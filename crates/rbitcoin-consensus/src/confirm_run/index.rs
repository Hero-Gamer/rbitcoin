//! Live filter and tweak bytes for one confirm batch.
//!
//! Jobs run on the script steal pool: one tweak per eligible transaction, one
//! filter per block. The scripts stage publishes them with the verify jobs.
//! No store read. [`Query::index_live`] is set at startup, after a short
//! restart gap is sealed, so load does not sample `next_height`. A write that
//! finds the watermark is not this batch is corrupt.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use bitcoin::hashes::Hash;
use bitcoin::{Amount, Block, ScriptBuf, Transaction, TxOut};
use rbitcoin_primitives::{Fk, Height};
use rbitcoin_query::{ParentPinView, Query};
use rbitcoin_store::StoreError;

use crate::index_rows::{self, IndexRows};
use crate::silent_payments::tweak_from_tx;
use crate::ConsensusError;

use super::{LoadedBatch, Prepared};

/// Which indexes this batch should assemble. Set at load from [`Query::index_live`].
#[derive(Clone, Debug, Default)]
pub(super) struct IndexWant {
    pub filters: bool,
    /// Tweaks for heights at or above this origin. `None` when tweaks are off.
    pub tweak_origin: Option<u32>,
}

/// One confirm batch of filter bytes and tweak vecs. Dropped with the batch.
pub(super) type IndexSeal = IndexRows;

/// Wall from batch start until every job of this kind has finished.
pub(super) struct KindClock {
    t0: Instant,
    left: AtomicUsize,
    ns: AtomicU64,
    stamped: AtomicBool,
}

impl KindClock {
    pub(super) fn new(t0: Instant, n: usize, empty_ns: u64) -> Arc<Self> {
        Arc::new(Self {
            t0,
            left: AtomicUsize::new(n),
            ns: AtomicU64::new(if n == 0 { empty_ns } else { 0 }),
            stamped: AtomicBool::new(n == 0),
        })
    }

    pub(super) fn job_done(&self) {
        if self.left.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.ns
                .store(self.t0.elapsed().as_nanos() as u64, Ordering::Release);
            self.stamped.store(true, Ordering::Release);
        }
    }

    pub(super) fn ns(&self) -> u64 {
        if self.stamped.load(Ordering::Acquire) {
            self.ns.load(Ordering::Acquire)
        } else {
            self.t0.elapsed().as_nanos() as u64
        }
    }
}

enum Written<T> {
    Pending,
    Ready(T),
}

type TweakSlots = Arc<[Mutex<Written<Option<[u8; 33]>>>]>;

#[derive(Clone, Copy)]
struct SpendRef {
    prev_txid: [u8; 32],
    vout: u32,
    create_fk: Fk,
    vin: u32,
}

struct BlockWork {
    block: Arc<Block>,
    hash: [u8; 32],
    parents: Arc<ParentPinView>,
    /// Spends grouped by tx index. Parallel jobs read their own slot.
    spends_by_tx: Arc<[Vec<SpendRef>]>,
    tx_fks_len: usize,
    same_block: OnceLock<HashMap<[u8; 32], usize>>,
    tweaks: TweakSlots,
    filter: Option<Arc<Mutex<Written<bitcoin::bip158::BlockFilter>>>>,
}

pub(super) struct HeightSlots {
    height: Height,
    header_fk: Fk,
    want_tweaks: bool,
    tweaks: TweakSlots,
    filter: Option<Arc<Mutex<Written<bitcoin::bip158::BlockFilter>>>>,
}

pub(super) struct TweakJob {
    work: Arc<BlockWork>,
    tx_index: usize,
    clock: Arc<KindClock>,
}

pub(super) struct FilterJob {
    work: Arc<BlockWork>,
    clock: Arc<KindClock>,
}

pub(super) struct IndexJobs {
    pub tweaks: Vec<TweakJob>,
    pub filters: Vec<FilterJob>,
    pub slots: Vec<HeightSlots>,
    pub clock: Arc<KindClock>,
}

pub(super) fn index_want(query: &Query) -> IndexWant {
    if !query.index_live() {
        return IndexWant::default();
    }
    IndexWant {
        filters: query.filter_index_next().is_some(),
        tweak_origin: query.tweak_index_next().map(|_| query.sptweaks_origin().0),
    }
}

pub(super) fn prepare_index_jobs(
    batch: &LoadedBatch,
    t0: Instant,
) -> Result<IndexJobs, ConsensusError> {
    let want = &batch.index_want;
    if !want.filters && want.tweak_origin.is_none() {
        return Ok(IndexJobs {
            tweaks: Vec::new(),
            filters: Vec::new(),
            slots: Vec::new(),
            clock: KindClock::new(t0, 0, 0),
        });
    }
    if batch.prepared.len() != batch.wire_blocks.len() {
        return Err(corrupt("invariant: live index block count"));
    }
    let parents = Arc::new(batch.batch_parents.pin_view());
    let mut tweaks = Vec::new();
    let mut filters = Vec::new();
    let mut slots = Vec::new();
    for (prep, block) in batch.prepared.iter().zip(batch.wire_blocks.iter()) {
        let want_tweaks = want.tweak_origin.is_some_and(|o| prep.height.0 >= o);
        if !want.filters && !want_tweaks {
            continue;
        }
        let spends_by_tx = group_spends(prep, block.txdata.len())?;
        let tweak_cells: TweakSlots = block
            .txdata
            .iter()
            .map(|tx| {
                let ready = !want_tweaks || tx.is_coinbase() || !tx_has_taproot_out(tx);
                Mutex::new(if ready {
                    Written::Ready(None)
                } else {
                    Written::Pending
                })
            })
            .collect::<Vec<_>>()
            .into();
        let filter = if want.filters {
            Some(Arc::new(Mutex::new(Written::Pending)))
        } else {
            None
        };
        let work = Arc::new(BlockWork {
            block: Arc::clone(block),
            hash: prep.hash,
            parents: Arc::clone(&parents),
            spends_by_tx: spends_by_tx.into(),
            tx_fks_len: prep.tx_fks.len(),
            same_block: OnceLock::new(),
            tweaks: Arc::clone(&tweak_cells),
            filter: filter.clone(),
        });
        if want_tweaks {
            for (ti, tx) in block.txdata.iter().enumerate() {
                if tx.is_coinbase() || !tx_has_taproot_out(tx) {
                    continue;
                }
                tweaks.push(TweakJob {
                    work: Arc::clone(&work),
                    tx_index: ti,
                    clock: KindClock::new(t0, 0, 0),
                });
            }
        }
        if filter.is_some() {
            filters.push(FilterJob {
                work,
                clock: KindClock::new(t0, 0, 0),
            });
        }
        slots.push(HeightSlots {
            height: prep.height,
            header_fk: prep.header_fk,
            want_tweaks,
            tweaks: tweak_cells,
            filter,
        });
    }
    let n_jobs = tweaks.len() + filters.len();
    let clock = KindClock::new(t0, n_jobs, 0);
    for job in &mut tweaks {
        job.clock = Arc::clone(&clock);
    }
    for job in &mut filters {
        job.clock = Arc::clone(&clock);
    }
    Ok(IndexJobs {
        tweaks,
        filters,
        slots,
        clock,
    })
}

pub(super) fn apply_tweak(job: &TweakJob) -> Result<(), ConsensusError> {
    #[cfg(test)]
    tweak_gate::enter();
    let tx = &job.work.block.txdata[job.tx_index];
    let prevouts = prevouts_for_tx(&job.work, job.tx_index, tx)?;
    let tweak = tweak_from_tx(tx, &prevouts).map(|t| t.tweak);
    *lock_written(&job.work.tweaks[job.tx_index]) = Written::Ready(tweak);
    job.clock.job_done();
    Ok(())
}

pub(super) fn apply_filter(job: &FilterJob) -> Result<(), ConsensusError> {
    let prevouts = prevouts_for_block(&job.work)?;
    let filter = basic_filter_from_wire(&job.work.hash, &job.work.block, &prevouts)?;
    let Some(slot) = job.work.filter.as_ref() else {
        return Err(corrupt("invariant: filter job"));
    };
    *lock_written(slot) = Written::Ready(filter);
    job.clock.job_done();
    Ok(())
}

pub(super) fn rows_from_heights(slots: &[HeightSlots]) -> Result<IndexRows, ConsensusError> {
    let mut rows = IndexRows::default();
    for height in slots {
        if let Some(slot) = &height.filter {
            rows.filters
                .push((height.height, take_written(slot)?, height.header_fk));
        }
        if !height.want_tweaks {
            continue;
        }
        let mut tweaks = Vec::with_capacity(height.tweaks.len());
        for cell in height.tweaks.iter() {
            tweaks.push(take_written(cell)?);
        }
        rows.tweaks.push((height.height, height.header_fk, tweaks));
    }
    Ok(rows)
}

fn lock_written<T>(cell: &Mutex<Written<T>>) -> std::sync::MutexGuard<'_, Written<T>> {
    cell.lock().unwrap_or_else(|e| e.into_inner())
}

fn take_written<T>(cell: &Mutex<Written<T>>) -> Result<T, ConsensusError> {
    match std::mem::replace(&mut *lock_written(cell), Written::Pending) {
        Written::Ready(value) => Ok(value),
        Written::Pending => Err(corrupt("invariant: index slot")),
    }
}

fn tx_has_taproot_out(tx: &Transaction) -> bool {
    tx.output.iter().any(|out| {
        let spk = out.script_pubkey.as_bytes();
        spk.len() == 34 && spk[0] == 0x51 && spk[1] == 0x20
    })
}

fn group_spends(prep: &Prepared, n_tx: usize) -> Result<Vec<Vec<SpendRef>>, ConsensusError> {
    let mut index_of: HashMap<Fk, usize> = HashMap::with_capacity(prep.tx_fks.len());
    for (i, fk) in prep.tx_fks.iter().copied().enumerate() {
        if i >= n_tx {
            break;
        }
        index_of.entry(fk).or_insert(i);
    }
    let mut by_tx = vec![Vec::new(); n_tx];
    for &(prev_txid, vout, spending_fk, create_fk, vin) in &prep.spends {
        if spending_fk.is_null() {
            return Err(corrupt("invariant: live index spend fk"));
        }
        let Some(&ti) = index_of.get(&spending_fk) else {
            return Err(corrupt("invariant: live index tx fk"));
        };
        by_tx[ti].push(SpendRef {
            prev_txid,
            vout,
            create_fk,
            vin,
        });
    }
    Ok(by_tx)
}

fn basic_filter_from_wire(
    hash: &[u8; 32],
    block: &Block,
    prevouts: &[Vec<TxOut>],
) -> Result<bitcoin::bip158::BlockFilter, ConsensusError> {
    let outputs = block
        .txdata
        .iter()
        .flat_map(|tx| tx.output.iter().map(|o| o.script_pubkey.as_bytes()));
    let spent = prevouts
        .iter()
        .flatten()
        .map(|prev| prev.script_pubkey.as_bytes());
    rbitcoin_query::basic_filter_from_scripts(hash, outputs, spent).map_err(ConsensusError::from)
}

/// Test latch for a tweak job. Armed only by the overlap test.
/// [`apply_tweak`] calls [`tweak_gate::enter`] under `cfg(test)`.
#[cfg(test)]
pub(in crate::confirm_run) mod tweak_gate {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Condvar, Mutex};
    use std::time::Duration;

    static ARMED: AtomicBool = AtomicBool::new(false);
    static ENTERED: AtomicUsize = AtomicUsize::new(0);
    static SCRIPT_WORKER: AtomicBool = AtomicBool::new(false);
    static RELEASED: AtomicBool = AtomicBool::new(false);
    static MU: Mutex<()> = Mutex::new(());
    static CV: Condvar = Condvar::new();

    pub(in crate::confirm_run) fn arm() {
        ENTERED.store(0, Ordering::SeqCst);
        SCRIPT_WORKER.store(false, Ordering::SeqCst);
        RELEASED.store(false, Ordering::SeqCst);
        ARMED.store(true, Ordering::SeqCst);
    }

    pub(in crate::confirm_run) fn enter() {
        if !ARMED.load(Ordering::SeqCst) {
            return;
        }
        let current = std::thread::current();
        let name = current.name().unwrap_or("");
        if name.starts_with("rbtc-scripts") {
            SCRIPT_WORKER.store(true, Ordering::SeqCst);
        }
        ENTERED.fetch_add(1, Ordering::SeqCst);
        CV.notify_all();
        let mut guard = MU.lock().unwrap_or_else(|p| p.into_inner());
        while !RELEASED.load(Ordering::SeqCst) {
            let (next, _) = CV
                .wait_timeout(guard, Duration::from_millis(20))
                .unwrap_or_else(|p| p.into_inner());
            guard = next;
        }
    }

    pub(in crate::confirm_run) fn entered() -> usize {
        ENTERED.load(Ordering::SeqCst)
    }

    pub(in crate::confirm_run) fn on_script_worker() -> bool {
        SCRIPT_WORKER.load(Ordering::SeqCst)
    }

    pub(in crate::confirm_run) fn release() {
        RELEASED.store(true, Ordering::SeqCst);
        ARMED.store(false, Ordering::SeqCst);
        CV.notify_all();
    }
}

fn prevouts_for_block(work: &BlockWork) -> Result<Vec<Vec<TxOut>>, ConsensusError> {
    let mut out = Vec::with_capacity(work.block.txdata.len());
    for (ti, tx) in work.block.txdata.iter().enumerate() {
        out.push(prevouts_for_tx(work, ti, tx)?);
    }
    Ok(out)
}

fn prevouts_for_tx(
    work: &BlockWork,
    ti: usize,
    tx: &Transaction,
) -> Result<Vec<TxOut>, ConsensusError> {
    if tx.is_coinbase() {
        return Ok(Vec::new());
    }
    if ti >= work.tx_fks_len {
        return Err(corrupt("invariant: live index tx fk"));
    }
    let Some(rows) = work.spends_by_tx.get(ti) else {
        return Err(corrupt("invariant: live index tx fk"));
    };
    let mut rows = rows.clone();
    rows.sort_by_key(|spend| spend.vin);
    if rows.len() != tx.input.len() {
        return Err(corrupt("invariant: live index prevout count"));
    }
    let mut out = Vec::with_capacity(rows.len());
    for spend in &rows {
        out.push(resolve_prevout(work, spend)?);
    }
    Ok(out)
}

fn resolve_prevout(work: &BlockWork, spend: &SpendRef) -> Result<TxOut, ConsensusError> {
    if spend.create_fk.is_null() {
        let same_block = work.same_block.get_or_init(|| {
            let mut map = HashMap::with_capacity(work.block.txdata.len());
            for (i, tx) in work.block.txdata.iter().enumerate() {
                map.insert(tx.compute_txid().to_byte_array(), i);
            }
            map
        });
        let Some(&pi) = same_block.get(&spend.prev_txid) else {
            return Err(corrupt("invariant: blockfilter prevout missing"));
        };
        let Some(out) = work.block.txdata[pi].output.get(spend.vout as usize) else {
            return Err(corrupt("invariant: blockfilter prevout missing"));
        };
        return Ok(out.clone());
    }
    let Some((value, script, txid)) = work.parents.txout_parts(spend.create_fk, spend.vout) else {
        return Err(corrupt("invariant: blockfilter prevout missing"));
    };
    if txid != spend.prev_txid {
        return Err(corrupt("invariant: live index prevout txid"));
    }
    if value < 0 {
        return Err(corrupt("invariant: live index prevout value"));
    }
    Ok(TxOut {
        value: Amount::from_sat(value as u64),
        script_pubkey: ScriptBuf::from_bytes(script),
    })
}

/// After `confirmed[h]` is set: commit this batch. A watermark that is not
/// the first row is corrupt. An empty seal (live append off) is a no-op.
pub(super) fn seal_live_indexes(query: &Query, seal: IndexSeal) -> Result<u64, ConsensusError> {
    if seal.filters.is_empty() && seal.tweaks.is_empty() {
        return Ok(0);
    }
    let commit = index_rows::commit_index_rows(query, seal)?;
    if commit.moved {
        return Err(corrupt("invariant: live index commit"));
    }
    Ok(commit.put_ns)
}

fn corrupt(msg: &'static str) -> ConsensusError {
    ConsensusError::Store(StoreError::Corrupt(msg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::absolute::LockTime;
    use bitcoin::hashes::{hash160, Hash};
    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
    use bitcoin::transaction::Version as TxVersion;
    use bitcoin::{Amount, OutPoint, Sequence, TxIn, TxOut, Witness};
    use rbitcoin_query::testutil::tiny_query_labeled;
    use rbitcoin_query::Query;

    use crate::silent_payments::tweak_records_from_window;
    use crate::{
        accept_and_connect_block, confirm_wire_run, genesis_block, mine_empty_regtest,
        mine_regtest_paying, ChainParams, Milestone,
    };

    fn keys() -> (Vec<u8>, bitcoin::ScriptBuf, bitcoin::ScriptBuf) {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[2u8; 32]).unwrap();
        let pk = PublicKey::from_secret_key(&secp, &sk);
        let ser = pk.serialize().to_vec();
        let h160 = hash160::Hash::hash(&ser);
        let mut p2wpkh = vec![0x00, 0x14];
        p2wpkh.extend_from_slice(h160.as_ref());
        let (xonly, _) = pk.x_only_public_key();
        let mut p2tr = vec![0x51, 0x20];
        p2tr.extend_from_slice(&xonly.serialize());
        (
            ser,
            bitcoin::ScriptBuf::from_bytes(p2wpkh),
            bitcoin::ScriptBuf::from_bytes(p2tr),
        )
    }

    fn enable(q: &Query) {
        q.set_block_filter_index(true).unwrap();
        q.set_sptweaks_enabled(true, Height(0)).unwrap();
    }

    fn go_live(q: &Query) {
        crate::prepare_live_indexes(q).unwrap();
    }

    fn assert_sealed_matches_window(q: &Query, tip: u32) {
        assert_eq!(q.filter_index_next(), Some(tip + 1));
        assert_eq!(q.tweak_index_next(), Some(tip + 1));
        for h in 0..=tip {
            let stored = q.basic_filter_at(h).unwrap().unwrap().0;
            let heights = q.index_heights(h, h, Some(0)).unwrap();
            let window = q.read_index_window(&heights).unwrap();
            let expect = q.basic_filter_from_window(&window, 0).unwrap();
            assert_eq!(stored, expect.content, "filter h={h}");
            let recs = tweak_records_from_window(&window, 0).unwrap();
            let want: Vec<[u8; 33]> = recs.into_iter().flatten().collect();
            let got = q.load_thin_tweaks(Height(h)).unwrap().unwrap_or_default();
            let got: Vec<[u8; 33]> = got.into_iter().map(|r| r.tweak).collect();
            assert_eq!(got, want, "tweaks h={h}");
        }
    }

    /// Three roles in one batch: a P2WPKH coinbase, a same-batch spend into
    /// P2TR once that coinbase is mature, and the blocks between them.
    #[test]
    fn live_batch_matches_window_and_reaches_tip() {
        let _gate = crate::script_pool::steal_test_gate();
        let (_dir, q) = tiny_query_labeled("idx-live");
        enable(&q);
        go_live(&q);
        let params = ChainParams::regtest();
        let ms = Milestone::height(u32::MAX);
        let genesis = genesis_block(&params);
        accept_and_connect_block(&q, &params, Height(0), &genesis, ms).unwrap();

        let (ser, p2wpkh, p2tr) = keys();
        let spend_h = 101u32;
        let mut blocks = Vec::new();
        let mut prev = genesis.block_hash();
        let mut time = genesis.header.time;
        let mut pay_value = Amount::ZERO;
        let mut pay_txid = None;
        for h in 1..=spend_h {
            time += 600;
            let block = if h == 1 {
                mine_regtest_paying(prev, time, h, p2wpkh.clone(), Vec::new())
            } else if h == spend_h {
                let txid = pay_txid.expect("paying coinbase");
                let spend = bitcoin::Transaction {
                    version: TxVersion::TWO,
                    lock_time: LockTime::ZERO,
                    input: vec![TxIn {
                        previous_output: OutPoint::new(txid, 0),
                        script_sig: bitcoin::ScriptBuf::new(),
                        sequence: Sequence::MAX,
                        witness: Witness::from_slice(&[vec![0u8; 64].as_slice(), ser.as_slice()]),
                    }],
                    output: vec![TxOut {
                        value: pay_value,
                        script_pubkey: p2tr.clone(),
                    }],
                };
                let child = bitcoin::Transaction {
                    version: TxVersion::TWO,
                    lock_time: LockTime::ZERO,
                    input: vec![TxIn {
                        previous_output: OutPoint::new(spend.compute_txid(), 0),
                        script_sig: bitcoin::ScriptBuf::new(),
                        sequence: Sequence::MAX,
                        witness: Witness::new(),
                    }],
                    output: vec![TxOut {
                        value: pay_value,
                        script_pubkey: p2tr.clone(),
                    }],
                };
                mine_regtest_paying(prev, time, h, p2wpkh.clone(), vec![spend, child])
            } else {
                mine_empty_regtest(prev, time, h)
            };
            if h == 1 {
                pay_value = block.txdata[0].output[0].value;
                pay_txid = Some(block.txdata[0].compute_txid());
            }
            prev = block.block_hash();
            time = block.header.time;
            blocks.push((Height(h), block));
        }
        confirm_wire_run(&q, &params, ms, &blocks).unwrap();
        assert_sealed_matches_window(&q, spend_h);
        let rows = q.load_thin_tweaks(Height(spend_h)).unwrap().unwrap();
        assert_eq!(
            rows.len(),
            2,
            "the P2TR spend and its same-block child are eligible"
        );
    }

    #[test]
    fn far_watermark_does_not_advance() {
        let _gate = crate::script_pool::steal_test_gate();
        let (_dir, q) = tiny_query_labeled("idx-gap");
        let params = ChainParams::regtest();
        let ms = Milestone::height(u32::MAX);
        let genesis = genesis_block(&params);
        accept_and_connect_block(&q, &params, Height(0), &genesis, ms).unwrap();
        let mut prev = genesis.block_hash();
        let mut time = genesis.header.time;
        for h in 1..=4 {
            time += 600;
            let block = mine_empty_regtest(prev, time, h);
            accept_and_connect_block(&q, &params, Height(h), &block, ms).unwrap();
            prev = block.block_hash();
            time = block.header.time;
        }
        enable(&q);
        crate::index_writebehind::prepare_live_indexes_limited(&q, 2).unwrap();
        assert!(!q.index_live());
        assert_eq!(q.filter_index_next(), Some(0));
        time += 600;
        let block = mine_empty_regtest(prev, time, 5);
        confirm_wire_run(&q, &params, ms, &[(Height(5), block)]).unwrap();
        assert_eq!(q.tip_height(), Some(Height(5)));
        assert_eq!(q.filter_index_next(), Some(0));
        assert_eq!(q.tweak_index_next(), Some(0));
    }

    #[test]
    fn startup_seals_a_short_gap_then_the_batch_appends() {
        let _gate = crate::script_pool::steal_test_gate();
        let (_dir, q) = tiny_query_labeled("idx-repair");
        let params = ChainParams::regtest();
        let ms = Milestone::height(u32::MAX);
        let genesis = genesis_block(&params);
        accept_and_connect_block(&q, &params, Height(0), &genesis, ms).unwrap();
        let b1 = mine_empty_regtest(genesis.block_hash(), genesis.header.time + 600, 1);
        accept_and_connect_block(&q, &params, Height(1), &b1, ms).unwrap();
        enable(&q);
        go_live(&q);
        assert!(q.index_live());
        assert_eq!(q.filter_index_next(), Some(2));
        let b2 = mine_empty_regtest(b1.block_hash(), b1.header.time + 600, 2);
        confirm_wire_run(&q, &params, ms, &[(Height(2), b2)]).unwrap();
        assert_sealed_matches_window(&q, 2);
    }

    /// Filters still at genesis and tweaks at a later origin are two holes.
    /// A limit that fits only the tweak hole does not turn live append on.
    #[test]
    fn startup_repairs_each_fitting_index() {
        let _gate = crate::script_pool::steal_test_gate();
        let (_dir, q) = tiny_query_labeled("idx-split");
        let params = ChainParams::regtest();
        let ms = Milestone::height(u32::MAX);
        let genesis = genesis_block(&params);
        accept_and_connect_block(&q, &params, Height(0), &genesis, ms).unwrap();
        let mut prev = genesis.block_hash();
        let mut time = genesis.header.time;
        for h in 1..=3 {
            time += 600;
            let block = mine_empty_regtest(prev, time, h);
            accept_and_connect_block(&q, &params, Height(h), &block, ms).unwrap();
            prev = block.block_hash();
            time = block.header.time;
        }
        q.set_block_filter_index(true).unwrap();
        q.set_sptweaks_enabled(true, Height(2)).unwrap();
        crate::index_writebehind::prepare_live_indexes_limited(&q, 2).unwrap();
        assert!(!q.index_live());
        assert_eq!(q.filter_index_next(), Some(0));
        assert_eq!(q.tweak_index_next(), Some(4));
        time += 600;
        let block = mine_empty_regtest(prev, time, 4);
        confirm_wire_run(&q, &params, ms, &[(Height(4), block)]).unwrap();
        assert_eq!(q.filter_index_next(), Some(0));
        assert_eq!(q.tweak_index_next(), Some(4));

        let (_dir, both) = tiny_query_labeled("idx-split-both");
        accept_and_connect_block(&both, &params, Height(0), &genesis, ms).unwrap();
        let b1 = mine_empty_regtest(genesis.block_hash(), genesis.header.time + 600, 1);
        accept_and_connect_block(&both, &params, Height(1), &b1, ms).unwrap();
        both.set_block_filter_index(true).unwrap();
        both.set_sptweaks_enabled(true, Height(0)).unwrap();
        go_live(&both);
        assert!(both.index_live());
        assert_sealed_matches_window(&both, 1);
    }

    #[test]
    fn tip_entry_spawns_only_while_a_watermark_is_behind() {
        let (_dir, q) = tiny_query_labeled("idx-tip");
        enable(&q);
        go_live(&q);
        let idle = crate::index_tip_entry(&q);
        assert!(!idle.spawn);
        assert!(!idle.advertise_filters);

        let params = ChainParams::regtest();
        let ms = Milestone::height(u32::MAX);
        let genesis = genesis_block(&params);
        accept_and_connect_block(&q, &params, Height(0), &genesis, ms).unwrap();
        let caught = crate::index_tip_entry(&q);
        assert!(!caught.spawn);
        assert!(caught.advertise_filters);

        let (_dir, behind) = tiny_query_labeled("idx-tip-behind");
        accept_and_connect_block(&behind, &params, Height(0), &genesis, ms).unwrap();
        enable(&behind);
        let gap = crate::index_tip_entry(&behind);
        assert!(gap.spawn);
        assert!(!gap.advertise_filters);
    }
}
