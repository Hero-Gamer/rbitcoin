//! Live filter and tweak bytes for one confirm batch.
//!
//! Assemble runs only while [`Query::index_live`] is set, from the wire block
//! and [`BatchParents`] (same-batch creates and pinned parents). No store read.
//! The flag is set at startup, after a short restart gap is sealed, so load
//! does not sample `next_height` (the previous batch can still be committing).
//! A write that finds the watermark is not this batch is corrupt.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bitcoin::hashes::Hash;
use bitcoin::{Amount, Block, ScriptBuf, TxOut};
use rbitcoin_primitives::{Fk, Height};
use rbitcoin_query::Query;
use rbitcoin_store::StoreError;

use crate::index_rows::{self, IndexHeightOut, IndexRows};
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

struct LiveJob {
    block: Arc<Block>,
    hash: [u8; 32],
    height: Height,
    header_fk: Fk,
    /// Per tx. Empty for a coinbase. Non-coinbase length matches `tx.input`.
    prevouts: Vec<Vec<TxOut>>,
    want_filter: bool,
    want_tweaks: bool,
    out: Arc<Mutex<Option<IndexHeightOut>>>,
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

pub(super) fn live_index_seal(batch: &LoadedBatch) -> Result<(IndexRows, u64), ConsensusError> {
    let want = &batch.index_want;
    if !want.filters && want.tweak_origin.is_none() {
        return Ok((IndexSeal::default(), 0));
    }
    let t0 = Instant::now();
    if batch.prepared.len() != batch.wire_blocks.len() {
        return Err(corrupt("invariant: live index block count"));
    }
    let mut slots = Vec::with_capacity(batch.prepared.len());
    let mut jobs = Vec::new();
    for (prep, block) in batch.prepared.iter().zip(batch.wire_blocks.iter()) {
        let want_tweaks = want.tweak_origin.is_some_and(|o| prep.height.0 >= o);
        let out = Arc::new(Mutex::new(None));
        if want.filters || want_tweaks {
            let prevouts = prevouts_for_block(prep, block, &batch.batch_parents)?;
            jobs.push(LiveJob {
                block: Arc::clone(block),
                hash: prep.hash,
                height: prep.height,
                header_fk: prep.header_fk,
                prevouts,
                want_filter: want.filters,
                want_tweaks,
                out: Arc::clone(&out),
            });
        }
        slots.push(out);
    }
    index_rows::finish_wave(jobs, assemble_live_height)?;
    let ns = t0.elapsed().as_nanos() as u64;
    Ok((index_rows::rows_from_slots(&slots), ns))
}

fn assemble_live_height(job: &LiveJob) -> Result<(), ConsensusError> {
    let filter = if job.want_filter {
        Some((
            job.height,
            basic_filter_from_wire(&job.hash, &job.block, &job.prevouts)?,
            job.header_fk,
        ))
    } else {
        None
    };
    let tweaks = if job.want_tweaks {
        Some((
            job.height,
            job.header_fk,
            tweaks_from_wire(&job.block, &job.prevouts),
        ))
    } else {
        None
    };
    *job.out.lock().unwrap_or_else(|e| e.into_inner()) = Some(IndexHeightOut { filter, tweaks });
    Ok(())
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

fn tweaks_from_wire(block: &Block, prevouts: &[Vec<TxOut>]) -> index_rows::HeightTweaks {
    block
        .txdata
        .iter()
        .zip(prevouts.iter())
        .map(|(tx, prev)| tweak_from_tx(tx, prev).map(|t| t.tweak))
        .collect()
}

fn prevouts_for_block(
    prep: &Prepared,
    block: &Block,
    parents: &rbitcoin_query::BatchParents,
) -> Result<Vec<Vec<TxOut>>, ConsensusError> {
    let mut same_block: HashMap<[u8; 32], usize> = HashMap::new();
    for (i, tx) in block.txdata.iter().enumerate() {
        same_block.insert(tx.compute_txid().to_byte_array(), i);
    }
    let mut by_tx: HashMap<Fk, Vec<(u32, TxOut)>> = HashMap::new();
    for &(prev_txid, vout, spending_fk, create_fk, vin) in &prep.spends {
        if spending_fk.is_null() {
            return Err(corrupt("invariant: live index spend fk"));
        }
        let txout = if create_fk.is_null() {
            let Some(&pi) = same_block.get(&prev_txid) else {
                return Err(corrupt("invariant: blockfilter prevout missing"));
            };
            let Some(o) = block.txdata[pi].output.get(vout as usize) else {
                return Err(corrupt("invariant: blockfilter prevout missing"));
            };
            o.clone()
        } else {
            let Some((value, script, txid)) =
                parents.get_parent_txout_parts(create_fk, vout, |v, s, txid| (v, s.to_vec(), txid))
            else {
                return Err(corrupt("invariant: blockfilter prevout missing"));
            };
            if txid != prev_txid {
                return Err(corrupt("invariant: live index prevout txid"));
            }
            if value < 0 {
                return Err(corrupt("invariant: live index prevout value"));
            }
            TxOut {
                value: Amount::from_sat(value as u64),
                script_pubkey: ScriptBuf::from_bytes(script),
            }
        };
        by_tx.entry(spending_fk).or_default().push((vin, txout));
    }
    let mut out = Vec::with_capacity(block.txdata.len());
    for (ti, tx) in block.txdata.iter().enumerate() {
        if tx.is_coinbase() {
            out.push(Vec::new());
            continue;
        }
        let fk = prep
            .tx_fks
            .get(ti)
            .copied()
            .ok_or_else(|| corrupt("invariant: live index tx fk"))?;
        let mut rows = by_tx.remove(&fk).unwrap_or_default();
        rows.sort_by_key(|(vin, _)| *vin);
        if rows.len() != tx.input.len() {
            return Err(corrupt("invariant: live index prevout count"));
        }
        out.push(rows.into_iter().map(|(_, o)| o).collect());
    }
    Ok(out)
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
