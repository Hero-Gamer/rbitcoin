//! Live filter and tweak bytes for one confirm batch.
//!
//! The scripts stage publishes these jobs on their own steal wave, after the
//! script wave. One job is one block filter or one range of at most
//! [`TWEAK_TXS_PER_JOB`] eligible transactions. Claim size is one job.
//! No store read. [`Query::index_live`] is set at startup, after a short
//! restart gap is sealed, so load does not sample `next_height`. A write that
//! finds the watermark is not this batch is corrupt.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use bitcoin::hashes::Hash;
use bitcoin::{Amount, Block, ScriptBuf, Transaction, TxOut};
use rbitcoin_primitives::{Fk, Height};
use rbitcoin_query::{ParentPinView, Query};
use rbitcoin_store::StoreError;

use crate::index_rows::{self, IndexRows};
use crate::silent_payments::{is_p2tr, tweak_from_tx};
use crate::ConsensusError;

use super::{LoadedBatch, Prepared};

/// Eligible transactions per tweak job. The index wave claims one job at a time.
pub(super) const TWEAK_TXS_PER_JOB: usize = 32;

/// Which indexes this batch should assemble. Set at load from [`Query::index_live`].
#[derive(Clone, Debug, Default)]
pub(super) struct IndexWant {
    pub filters: bool,
    /// Tweaks for heights at or above this origin. `None` when tweaks are off.
    pub tweak_origin: Option<u32>,
}

/// One confirm batch of filter bytes and tweak vecs. Dropped with the batch.
pub(super) type IndexSeal = IndexRows;

type TweakSlots = Arc<[OnceLock<Option<[u8; 33]>>]>;
type PrevoutSlots = Arc<[OnceLock<Vec<TxOut>>]>;

#[derive(Clone, Copy)]
struct SpendRef {
    prev_txid: [u8; 32],
    vout: u32,
    create_fk: Fk,
    vin: u32,
}

pub(super) struct BlockWork {
    block: Arc<Block>,
    hash: [u8; 32],
    parents: Arc<ParentPinView>,
    /// Spends grouped by tx index, sorted by `vin`. Parallel jobs read their own slot.
    spends_by_tx: Arc<[Vec<SpendRef>]>,
    same_block: OnceLock<HashMap<[u8; 32], usize>>,
    /// One cell per transaction. The filter and a tweak range share it.
    prevouts: PrevoutSlots,
    tweaks: TweakSlots,
    filter: Option<Arc<OnceLock<bitcoin::bip158::BlockFilter>>>,
    /// Shared by every block in the batch. The first index job wins.
    started: Arc<OnceLock<Instant>>,
}

pub(super) struct HeightSlots {
    height: Height,
    header_fk: Fk,
    want_tweaks: bool,
    tweaks: TweakSlots,
    filter: Option<Arc<OnceLock<bitcoin::bip158::BlockFilter>>>,
}

pub(super) enum IndexJob {
    Filter(Arc<BlockWork>),
    Tweaks(Arc<BlockWork>, Vec<usize>),
}

pub(super) struct IndexJobs {
    pub jobs: Vec<IndexJob>,
    pub slots: Vec<HeightSlots>,
    /// First index job writes this. The publisher reads it when the wave completes.
    pub started: Arc<OnceLock<Instant>>,
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

pub(super) fn prepare_index_jobs(batch: &LoadedBatch) -> Result<IndexJobs, ConsensusError> {
    let want = &batch.index_want;
    if !want.filters && want.tweak_origin.is_none() {
        return Ok(IndexJobs {
            jobs: Vec::new(),
            slots: Vec::new(),
            started: Arc::new(OnceLock::new()),
        });
    }
    if batch.prepared.len() != batch.wire_blocks.len() {
        return Err(corrupt("invariant: live index block count"));
    }
    let parents = Arc::new(batch.batch_parents.pin_view());
    let started = Arc::new(OnceLock::new());
    let mut jobs = Vec::new();
    let mut tweak_jobs = Vec::new();
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
                let cell = OnceLock::new();
                if !want_tweaks || tx.is_coinbase() || !tx_has_taproot_out(tx) {
                    let _ = cell.set(None);
                }
                cell
            })
            .collect::<Vec<_>>()
            .into();
        let n_tx = block.txdata.len();
        let prevouts: PrevoutSlots = (0..n_tx)
            .map(|_| OnceLock::new())
            .collect::<Vec<_>>()
            .into();
        let filter = if want.filters {
            Some(Arc::new(OnceLock::new()))
        } else {
            None
        };
        let work = Arc::new(BlockWork {
            block: Arc::clone(block),
            hash: prep.hash,
            parents: Arc::clone(&parents),
            spends_by_tx: spends_by_tx.into(),
            same_block: OnceLock::new(),
            prevouts,
            tweaks: Arc::clone(&tweak_cells),
            filter: filter.clone(),
            started: Arc::clone(&started),
        });
        if filter.is_some() {
            jobs.push(IndexJob::Filter(Arc::clone(&work)));
        }
        if want_tweaks {
            let eligible: Vec<usize> = block
                .txdata
                .iter()
                .enumerate()
                .filter(|(_, tx)| !tx.is_coinbase() && tx_has_taproot_out(tx))
                .map(|(ti, _)| ti)
                .collect();
            for range in eligible.chunks(TWEAK_TXS_PER_JOB) {
                tweak_jobs.push(IndexJob::Tweaks(Arc::clone(&work), range.to_vec()));
            }
        }
        slots.push(HeightSlots {
            height: prep.height,
            header_fk: prep.header_fk,
            want_tweaks,
            tweaks: tweak_cells,
            filter,
        });
    }
    jobs.extend(tweak_jobs);
    Ok(IndexJobs {
        jobs,
        slots,
        started,
    })
}

pub(super) fn apply_index_job(job: &IndexJob) -> Result<(), ConsensusError> {
    match job {
        IndexJob::Filter(work) => {
            let _ = work.started.set(Instant::now());
            apply_filter(work)
        }
        IndexJob::Tweaks(work, txs) => {
            let _ = work.started.set(Instant::now());
            apply_tweak_range(work, txs)
        }
    }
}

fn apply_tweak_range(work: &BlockWork, txs: &[usize]) -> Result<(), ConsensusError> {
    for &ti in txs {
        let tx = &work.block.txdata[ti];
        let prevouts = prevouts_cached(work, ti)?;
        let tweak = tweak_from_tx(tx, prevouts).map(|t| t.tweak);
        if work.tweaks[ti].set(tweak).is_err() {
            return Err(corrupt("invariant: index slot"));
        }
    }
    Ok(())
}

fn apply_filter(work: &BlockWork) -> Result<(), ConsensusError> {
    let mut prevouts = Vec::with_capacity(work.block.txdata.len());
    for ti in 0..work.block.txdata.len() {
        prevouts.push(prevouts_cached(work, ti)?.clone());
    }
    let filter = basic_filter_from_wire(&work.hash, &work.block, &prevouts)?;
    let Some(slot) = work.filter.as_ref() else {
        return Err(corrupt("invariant: filter job"));
    };
    if slot.set(filter).is_err() {
        return Err(corrupt("invariant: filter job"));
    }
    Ok(())
}

pub(super) fn rows_from_heights(slots: &[HeightSlots]) -> Result<IndexRows, ConsensusError> {
    let mut rows = IndexRows::default();
    for height in slots {
        if let Some(slot) = &height.filter {
            let Some(filter) = slot.get().cloned() else {
                return Err(corrupt("invariant: index slot"));
            };
            rows.filters.push((height.height, filter, height.header_fk));
        }
        if !height.want_tweaks {
            continue;
        }
        let mut tweaks = Vec::with_capacity(height.tweaks.len());
        for cell in height.tweaks.iter() {
            let Some(tweak) = cell.get().copied() else {
                return Err(corrupt("invariant: index slot"));
            };
            tweaks.push(tweak);
        }
        rows.tweaks.push((height.height, height.header_fk, tweaks));
    }
    Ok(rows)
}

fn tx_has_taproot_out(tx: &Transaction) -> bool {
    tx.output
        .iter()
        .any(|out| is_p2tr(out.script_pubkey.as_bytes()))
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
    for row in &mut by_tx {
        row.sort_by_key(|spend| spend.vin);
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

fn prevouts_cached(work: &BlockWork, ti: usize) -> Result<&Vec<TxOut>, ConsensusError> {
    if let Some(got) = work.prevouts[ti].get() {
        return Ok(got);
    }
    let built = prevouts_for_tx(work, ti, &work.block.txdata[ti])?;
    let _ = work.prevouts[ti].set(built);
    work.prevouts[ti]
        .get()
        .ok_or_else(|| corrupt("invariant: blockfilter prevout missing"))
}

fn prevouts_for_tx(
    work: &BlockWork,
    ti: usize,
    tx: &Transaction,
) -> Result<Vec<TxOut>, ConsensusError> {
    if tx.is_coinbase() {
        return Ok(Vec::new());
    }
    let rows = &work.spends_by_tx[ti];
    if rows.len() != tx.input.len() {
        return Err(corrupt("invariant: live index prevout count"));
    }
    let mut out = Vec::with_capacity(rows.len());
    for spend in rows {
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

    /// Filters come first, one per block. Tweaks are ranges of at most 32
    /// eligible transactions. A coinbase and a transaction with no Taproot
    /// output are not jobs. A prevout cell filled before either job runs is
    /// what both the filter and the tweak range read.
    #[test]
    fn index_jobs_are_filters_then_tweak_ranges() {
        let (ser, p2wpkh, p2tr) = keys();
        let batch = two_block_index_batch(&ser, &p2wpkh, &p2tr);
        let jobs = prepare_index_jobs(&batch).expect("prepare");
        let mut lens = Vec::new();
        for job in &jobs.jobs {
            match job {
                IndexJob::Filter(_) => lens.push(0),
                IndexJob::Tweaks(work, txs) => {
                    assert!(txs.len() <= TWEAK_TXS_PER_JOB);
                    for &ti in txs {
                        let tx = &work.block.txdata[ti];
                        assert!(!tx.is_coinbase(), "coinbase is not a tweak job");
                        assert!(
                            tx_has_taproot_out(tx),
                            "a transaction with no Taproot output is not a tweak job"
                        );
                    }
                    lens.push(txs.len());
                }
            }
        }
        // Two filters, then 40 and 33 eligible txs split at 32.
        assert_eq!(lens, vec![0, 0, 32, 8, 32, 1]);

        let IndexJob::Filter(filter0) = &jobs.jobs[0] else {
            panic!("first job is a filter");
        };
        let IndexJob::Tweaks(tweak0, _) = &jobs.jobs[2] else {
            panic!("tweak range follows both filters");
        };
        assert!(
            Arc::ptr_eq(filter0, tweak0),
            "filter and tweak share prevouts"
        );

        let ti = 1usize;
        let tx = &filter0.block.txdata[ti];
        let real = prevouts_for_tx(filter0, ti, tx).expect("prevouts");
        assert!(tweak_from_tx(tx, &real).is_some());
        let wrong = vec![TxOut {
            value: real[0].value,
            script_pubkey: bitcoin::ScriptBuf::new(),
        }];
        assert!(tweak_from_tx(tx, &wrong).is_none());
        filter0.prevouts[ti].set(wrong.clone()).expect("seed once");

        for job in &jobs.jobs {
            apply_index_job(job).expect("apply");
        }
        assert_eq!(filter0.prevouts[ti].get(), Some(&wrong));

        let rows = rows_from_heights(&jobs.slots).expect("rows");
        assert_eq!(rows.tweaks.len(), 2);
        assert_eq!(rows.filters.len(), 2);
        for (block_i, (_h, _fk, got)) in rows.tweaks.iter().enumerate() {
            let block = &batch.wire_blocks[block_i];
            let expect = expected_tweaks(
                block,
                if block_i == 0 {
                    Some((ti, &wrong))
                } else {
                    None
                },
            );
            assert_eq!(got, &expect, "tweaks block {block_i}");
        }
        let mut prevs = Vec::new();
        for (i, tx) in filter0.block.txdata.iter().enumerate() {
            if tx.is_coinbase() {
                prevs.push(Vec::new());
            } else if i == ti {
                prevs.push(wrong.clone());
            } else {
                let vout = tx.input[0].previous_output.vout as usize;
                prevs.push(vec![filter0.block.txdata[0].output[vout].clone()]);
            }
        }
        let expect_filter =
            basic_filter_from_wire(&filter0.hash, &filter0.block, &prevs).expect("filter");
        assert_eq!(&rows.filters[0].1, &expect_filter);
    }

    fn expected_tweaks(
        block: &bitcoin::Block,
        seeded: Option<(usize, &[TxOut])>,
    ) -> Vec<Option<[u8; 33]>> {
        block
            .txdata
            .iter()
            .enumerate()
            .map(|(i, tx)| {
                if tx.is_coinbase() {
                    return None;
                }
                let prev = if seeded.is_some_and(|(ti, _)| ti == i) {
                    seeded.expect("seed").1.to_vec()
                } else {
                    let vout = tx.input[0].previous_output.vout as usize;
                    vec![block.txdata[0].output[vout].clone()]
                };
                tweak_from_tx(tx, &prev).map(|t| t.tweak)
            })
            .collect()
    }

    fn taproot_spend(
        prev: bitcoin::Txid,
        vout: u32,
        spk: &bitcoin::ScriptBuf,
        ser: &[u8],
    ) -> Transaction {
        Transaction {
            version: TxVersion::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::new(prev, vout),
                script_sig: bitcoin::ScriptBuf::new(),
                sequence: Sequence::MAX,
                witness: Witness::from_slice(&[vec![0u8; 64].as_slice(), ser]),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(1_0000_0000),
                script_pubkey: spk.clone(),
            }],
        }
    }

    fn bare_block(txdata: Vec<Transaction>) -> bitcoin::Block {
        use bitcoin::block::{Header, Version};
        use bitcoin::{BlockHash, CompactTarget, TxMerkleNode};
        bitcoin::Block {
            header: Header {
                version: Version::from_consensus(4),
                prev_blockhash: BlockHash::from_byte_array([1; 32]),
                merkle_root: TxMerkleNode::from_byte_array([2; 32]),
                time: 1_700_000_000,
                bits: CompactTarget::from_consensus(0x207f_ffff),
                nonce: 0,
            },
            txdata,
        }
    }

    /// Two blocks, filters and tweaks on. Block 0 has 40 Taproot spends and
    /// one non-Taproot spend. Block 1 has 33 Taproot spends.
    fn two_block_index_batch(
        ser: &[u8],
        p2wpkh: &bitcoin::ScriptBuf,
        p2tr: &bitcoin::ScriptBuf,
    ) -> LoadedBatch {
        let (b0, p0) = index_block(10, 40, ser, p2wpkh, p2tr);
        let (b1, p1) = index_block(11, 33, ser, p2wpkh, p2tr);
        LoadedBatch {
            prepared: vec![p0, p1],
            wire_blocks: vec![Arc::new(b0), Arc::new(b1)],
            batch_parents: rbitcoin_query::BatchParents::new(),
            script_preverified: super::super::ScriptPreverified::new(),
            archive_plan: None,
            index_want: IndexWant {
                filters: true,
                tweak_origin: Some(0),
            },
            stats: Arc::new(rbitcoin_query::ConfirmStats::default()),
        }
    }

    fn index_block(
        height: u32,
        n_taproot: usize,
        ser: &[u8],
        p2wpkh: &bitcoin::ScriptBuf,
        p2tr: &bitcoin::ScriptBuf,
    ) -> (bitcoin::Block, Prepared) {
        let n_out = n_taproot + 1;
        let cb = Transaction {
            version: TxVersion::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: bitcoin::ScriptBuf::from_bytes(vec![0x01, height as u8]),
                sequence: Sequence::MAX,
                witness: Witness::new(),
            }],
            output: (0..n_out)
                .map(|_| TxOut {
                    value: Amount::from_sat(2_0000_0000),
                    script_pubkey: p2wpkh.clone(),
                })
                .collect(),
        };
        let cb_txid = cb.compute_txid();
        let mut txs = vec![cb];
        for vout in 0..n_taproot {
            txs.push(taproot_spend(cb_txid, vout as u32, p2tr, ser));
        }
        txs.push(taproot_spend(cb_txid, n_taproot as u32, p2wpkh, ser));
        let mut tx_fks = Vec::with_capacity(txs.len());
        let mut spends = Vec::new();
        for i in 0..txs.len() {
            tx_fks.push(Fk(1 + i as u64 + u64::from(height) * 100));
        }
        let cb_bytes = cb_txid.to_byte_array();
        for (i, tx) in txs.iter().enumerate().skip(1) {
            let vout = tx.input[0].previous_output.vout;
            spends.push((cb_bytes, vout, tx_fks[i], Fk::NULL, 0));
        }
        let block = bare_block(txs);
        let prep = Prepared {
            height: Height(height),
            header_fk: Fk(7 + u64::from(height)),
            tx_fks,
            jobs: Vec::new(),
            spends,
            fees: 0,
            check_scripts: false,
            time: block.header.time,
            bits: block.header.bits,
            hash: [height as u8; 32],
            prev_mtp: 0,
        };
        (block, prep)
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
