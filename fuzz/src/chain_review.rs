//! Hub-level chain differentials for the review discriminators.
//!
//! Each shape runs on a real [`ChainHub`]. The caller supplies Core's reply
//! (a live `submitblock` oracle, or a test double). A different fate is a
//! disagreement. These shapes do not skip.

use bitcoin::absolute::LockTime;
use bitcoin::block::{Header, Version};
use bitcoin::consensus::encode::serialize;
use bitcoin::hashes::Hash;
use bitcoin::transaction::Version as TxVersion;
use bitcoin::{
    Amount, Block, CompactTarget, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxMerkleNode,
    TxOut, Witness,
};
use rbitcoin_consensus::{
    block_subsidy, confirm_scripts_phase, genesis_block, grind_regtest_pow, mine_empty_regtest,
    mine_regtest_paying, ChainParams, Milestone, REGTEST_BLOCK_SPACING, REGTEST_POW_BITS,
};
use rbitcoin_net::{AcceptOutcome, ChainHub, NetError};
use rbitcoin_primitives::Height;

use crate::block_diff::{BlockOracle, OracleReply};

const SHAPE_MILESTONE: u8 = 0;
const SHAPE_GENESIS: u8 = 1;
const SHAPE_BIP30: u8 = 2;
const SHAPE_IMMATURE: u8 = 3;
const SHAPE_CSV: u8 = 4;
const SHAPE_MUTATION: u8 = 5;
const SHAPE_REORG: u8 = 6;
pub const CHAIN_REVIEW_SHAPES: u8 = 7;

/// High enough that regtest BIP34 stays off, matching a bitcoind started with
/// `-testactivationheight=bip34@100000000`. Coinbase scriptSigs may still push
/// a height; they just are not required to.
const BIP34_OFF: u32 = 100_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fate {
    Accept,
    /// Rejected as mutated and not cached as consensus-invalid.
    Mutated,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedSubmit {
    pub hex: String,
    /// `None` for blocks that only advance the chain. Compared submits carry a fate.
    pub(crate) fate: Option<Fate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainReview {
    Agree { accept: bool },
    Disagree { ours: bool, core: bool },
}

fn params_bip34_off() -> ChainParams {
    let mut params = ChainParams::regtest();
    params
        .apply_test_activation_height("bip34", BIP34_OFF)
        .expect("regtest overlay");
    params
}

fn hex_block(block: &Block) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let raw = serialize(block);
    let mut out = String::with_capacity(raw.len() * 2);
    for b in raw {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

fn op_true() -> ScriptBuf {
    ScriptBuf::from_bytes(vec![0x51])
}

fn subsidy(params: &ChainParams, height: u32) -> Amount {
    Amount::from_sat(block_subsidy(height, params) as u64)
}

fn spend_tx(prev: OutPoint, value: Amount, sequence: Sequence) -> Transaction {
    Transaction {
        version: TxVersion::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: prev,
            script_sig: ScriptBuf::new(),
            sequence,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value,
            script_pubkey: op_true(),
        }],
    }
}

fn hub_fate(hub: &ChainHub, block: Block) -> Result<Fate, String> {
    let hash = block.block_hash();
    match hub.accept_received_block(block) {
        Ok(_) => Ok(Fate::Accept),
        Err(NetError::Mutated(_)) => {
            if hub.is_block_invalid(&hash) {
                Ok(Fate::Reject)
            } else {
                Ok(Fate::Mutated)
            }
        }
        Err(
            NetError::Consensus(_)
            | NetError::ConnectFailed { .. }
            | NetError::BadPrev
            | NetError::SideBlock
            | NetError::UnknownParent,
        ) => Ok(Fate::Reject),
        Err(e) => Err(format!("hub: {e}")),
    }
}

fn push_setup(out: &mut Vec<PlannedSubmit>, block: &Block) {
    out.push(PlannedSubmit {
        hex: hex_block(block),
        fate: None,
    });
}

fn push_check(out: &mut Vec<PlannedSubmit>, block: &Block, fate: Fate) {
    out.push(PlannedSubmit {
        hex: hex_block(block),
        fate: Some(fate),
    });
}

struct HubSession {
    dir: rbitcoin_query::testutil::TempDir,
    hub: ChainHub,
}

impl Drop for HubSession {
    fn drop(&mut self) {
        let _keep = &self.dir;
    }
}

impl HubSession {
    fn open(label: &str, params: ChainParams, milestone: Milestone) -> Result<Self, String> {
        let (dir, q) = rbitcoin_query::testutil::tiny_query_labeled(label);
        let hub = ChainHub::new(q, params.clone(), milestone);
        hub.ensure_genesis().map_err(|e| format!("genesis: {e}"))?;
        Ok(Self { dir, hub })
    }
}

fn genesis_tip(params: &ChainParams) -> (bitcoin::BlockHash, u32) {
    let g = genesis_block(params);
    (g.block_hash(), g.header.time)
}

/// Run one review shape on a fresh hub. `shape` is wrapped into the seven discriminators.
pub fn plan_chain_shape(shape: u8) -> Result<Vec<PlannedSubmit>, String> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    match shape % CHAIN_REVIEW_SHAPES {
        SHAPE_MILESTONE => plan_milestone(),
        SHAPE_GENESIS => plan_genesis_spend(),
        SHAPE_BIP30 => plan_bip30(),
        SHAPE_IMMATURE => plan_immature_batch(),
        SHAPE_CSV => plan_height0_csv(),
        SHAPE_MUTATION => plan_mutation(),
        SHAPE_REORG => plan_reorg_respend(),
        _ => unreachable!(),
    }
}

fn plan_milestone() -> Result<Vec<PlannedSubmit>, String> {
    let params = params_bip34_off();
    // Height-only milestone (no anchor). The spend is under that height and
    // after coinbase maturity, so immaturity is not what decides the block.
    let maturity = params.coinbase_maturity();
    let spend_h = maturity + 1;
    let session = HubSession::open("chain-ms", params.clone(), Milestone::height(spend_h + 50))?;
    let (mut tip, mut time) = genesis_tip(&params);
    time += REGTEST_BLOCK_SPACING;
    let coin = mine_regtest_paying(tip, time, 1, ScriptBuf::from_bytes(vec![0x00]), Vec::new());
    let fate_coin = hub_fate(&session.hub, coin.clone())?;
    if fate_coin != Fate::Accept {
        return Err(format!("milestone coinbase rejected: {fate_coin:?}"));
    }
    let value = coin.txdata[0].output[0].value;
    let prev = OutPoint {
        txid: coin.txdata[0].compute_txid(),
        vout: 0,
    };
    let mut out = Vec::new();
    push_setup(&mut out, &coin);
    tip = coin.block_hash();
    time = coin.header.time;
    for h in 2..spend_h {
        time += REGTEST_BLOCK_SPACING;
        let b = mine_empty_regtest(tip, time, h);
        let fate = hub_fate(&session.hub, b.clone())?;
        if fate != Fate::Accept {
            return Err(format!("milestone pad {h}: {fate:?}"));
        }
        push_setup(&mut out, &b);
        tip = b.block_hash();
        time = b.header.time;
    }
    time += REGTEST_BLOCK_SPACING;
    let spend = mine_regtest_paying(
        tip,
        time,
        spend_h,
        op_true(),
        vec![spend_tx(prev, value, Sequence::MAX)],
    );
    let fate = hub_fate(&session.hub, spend.clone())?;
    push_check(&mut out, &spend, fate);
    drop(session);
    Ok(out)
}

fn plan_genesis_spend() -> Result<Vec<PlannedSubmit>, String> {
    let params = params_bip34_off();
    let session = HubSession::open("chain-gen", params.clone(), Milestone::NONE)?;
    let g = genesis_block(&params);
    let prev = OutPoint {
        txid: g.txdata[0].compute_txid(),
        vout: 0,
    };
    let value = g.txdata[0].output[0].value;
    let (tip, time) = genesis_tip(&params);
    let b = mine_regtest_paying(
        tip,
        time + REGTEST_BLOCK_SPACING,
        1,
        op_true(),
        vec![spend_tx(prev, value, Sequence::MAX)],
    );
    let fate = hub_fate(&session.hub, b.clone())?;
    let mut out = Vec::new();
    push_check(&mut out, &b, fate);
    drop(session);
    Ok(out)
}

fn plan_bip30() -> Result<Vec<PlannedSubmit>, String> {
    let params = params_bip34_off();
    let session = HubSession::open("chain-bip30", params.clone(), Milestone::NONE)?;
    let (tip, time) = genesis_tip(&params);
    let value = subsidy(&params, 1);
    let cb = duplicate_coinbase(value);
    let b1 = mine_single(&cb, tip, time + REGTEST_BLOCK_SPACING);
    let fate1 = hub_fate(&session.hub, b1.clone())?;
    if fate1 != Fate::Accept {
        return Err(format!("bip30 first block: {fate1:?}"));
    }
    let b2 = mine_single(&cb, b1.block_hash(), b1.header.time + REGTEST_BLOCK_SPACING);
    let fate2 = hub_fate(&session.hub, b2.clone())?;
    let mut out = Vec::new();
    push_setup(&mut out, &b1);
    push_check(&mut out, &b2, fate2);
    drop(session);
    Ok(out)
}

fn duplicate_coinbase(value: Amount) -> Transaction {
    Transaction {
        version: TxVersion::ONE,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::from_bytes(vec![0x01, 0x04]),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value,
            script_pubkey: ScriptBuf::from_bytes(vec![0x6a]),
        }],
    }
}

fn mine_single(tx: &Transaction, prev: bitcoin::BlockHash, time: u32) -> Block {
    let mut block = Block {
        header: Header {
            version: Version::from_consensus(4),
            prev_blockhash: prev,
            merkle_root: TxMerkleNode::from_byte_array(tx.compute_txid().to_byte_array()),
            time,
            bits: CompactTarget::from_consensus(REGTEST_POW_BITS),
            nonce: 0,
        },
        txdata: vec![tx.clone()],
    };
    grind_regtest_pow(&mut block.header);
    block
}

fn plan_immature_batch() -> Result<Vec<PlannedSubmit>, String> {
    let params = params_bip34_off();
    let session = HubSession::open("chain-imm", params.clone(), Milestone::NONE)?;
    let (tip, time) = genesis_tip(&params);
    let b1 = mine_regtest_paying(tip, time + REGTEST_BLOCK_SPACING, 1, op_true(), Vec::new());
    let value = b1.txdata[0].output[0].value;
    let prev = OutPoint {
        txid: b1.txdata[0].compute_txid(),
        vout: 0,
    };
    let b2 = mine_regtest_paying(
        b1.block_hash(),
        b1.header.time + REGTEST_BLOCK_SPACING,
        2,
        op_true(),
        vec![spend_tx(prev, value, Sequence::MAX)],
    );
    let accepted = batch_accept(
        &session.hub,
        &[(Height(1), b1.clone()), (Height(2), b2.clone())],
    )?;
    let fate = if accepted { Fate::Accept } else { Fate::Reject };
    let mut out = Vec::new();
    push_setup(&mut out, &b1);
    push_check(&mut out, &b2, fate);
    drop(session);
    Ok(out)
}

fn batch_accept(hub: &ChainHub, blocks: &[(Height, Block)]) -> Result<bool, String> {
    match hub.confirm_wire_load_phase(blocks) {
        Ok(None) => Ok(false),
        Ok(Some(loaded)) => match confirm_scripts_phase(loaded.batch) {
            Ok(scripts) => match hub.confirm_write(scripts.batch) {
                Ok(_) => Ok(true),
                Err(NetError::Consensus(_) | NetError::Mutated(_)) => Ok(false),
                Err(e) => Err(format!("batch write: {e}")),
            },
            Err(_) => Ok(false),
        },
        Err(NetError::Consensus(_) | NetError::Mutated(_)) => Ok(false),
        Err(e) => Err(format!("batch load: {e}")),
    }
}

fn plan_height0_csv() -> Result<Vec<PlannedSubmit>, String> {
    let params = params_bip34_off();
    let session = HubSession::open("chain-csv", params.clone(), Milestone::NONE)?;
    let g = genesis_block(&params);
    let prev = OutPoint {
        txid: g.txdata[0].compute_txid(),
        vout: 0,
    };
    let value = g.txdata[0].output[0].value;
    // BIP68 time-based lock. A panic in the height-0 median-time path propagates.
    let sequence = Sequence::from_consensus(1 << 22);
    let (tip, time) = genesis_tip(&params);
    let b = mine_regtest_paying(
        tip,
        time + REGTEST_BLOCK_SPACING,
        1,
        op_true(),
        vec![spend_tx(prev, value, sequence)],
    );
    let fate = hub_fate(&session.hub, b.clone())?;
    let mut out = Vec::new();
    push_check(&mut out, &b, fate);
    drop(session);
    Ok(out)
}

fn plan_mutation() -> Result<Vec<PlannedSubmit>, String> {
    let params = params_bip34_off();
    let session = HubSession::open("chain-mut", params.clone(), Milestone::NONE)?;
    let (tip, time) = genesis_tip(&params);
    let honest = mine_regtest_paying(tip, time + REGTEST_BLOCK_SPACING, 1, op_true(), Vec::new());
    let mut bad = honest.clone();
    bad.txdata[0].input[0].script_sig = ScriptBuf::from_bytes(vec![0x00]);
    let fate_bad = hub_fate(&session.hub, bad.clone())?;
    let fate_honest = hub_fate(&session.hub, honest.clone())?;
    let mut out = Vec::new();
    push_check(&mut out, &bad, fate_bad);
    push_check(&mut out, &honest, fate_honest);
    drop(session);
    Ok(out)
}

fn plan_reorg_respend() -> Result<Vec<PlannedSubmit>, String> {
    let params = params_bip34_off();
    let session = HubSession::open("chain-reorg", params.clone(), Milestone::NONE)?;
    let (mut tip, mut time) = genesis_tip(&params);
    let mut h1_out: Option<(OutPoint, Amount)> = None;
    let mut at_100: Option<(bitcoin::BlockHash, u32)> = None;
    let mut out = Vec::new();
    for h in 1..=100 {
        time += REGTEST_BLOCK_SPACING;
        let b = mine_empty_regtest(tip, time, h);
        if h == 1 {
            h1_out = Some((
                OutPoint {
                    txid: b.txdata[0].compute_txid(),
                    vout: 0,
                },
                b.txdata[0].output[0].value,
            ));
        }
        let fate = hub_fate(&session.hub, b.clone())?;
        if fate != Fate::Accept {
            return Err(format!("pad {h}: {fate:?}"));
        }
        push_setup(&mut out, &b);
        if h == 100 {
            at_100 = Some((b.block_hash(), b.header.time));
        }
        tip = b.block_hash();
        time = b.header.time;
    }
    let (prev, value) = h1_out.ok_or("missing height-1 coin")?;
    time += REGTEST_BLOCK_SPACING;
    let spend = mine_regtest_paying(
        tip,
        time,
        101,
        op_true(),
        vec![spend_tx(prev, value, Sequence::MAX)],
    );
    let fate_spend = hub_fate(&session.hub, spend.clone())?;
    if fate_spend != Fate::Accept {
        return Err(format!("mature spend: {fate_spend:?}"));
    }
    push_setup(&mut out, &spend);
    let (parent, parent_time) = at_100.ok_or("missing height 100")?;
    let fork_a = mine_empty_regtest(parent, parent_time + REGTEST_BLOCK_SPACING, 101);
    let fork_b = mine_empty_regtest(
        fork_a.block_hash(),
        fork_a.header.time + REGTEST_BLOCK_SPACING,
        102,
    );
    match session.hub.accept_branch(&[fork_a.clone(), fork_b.clone()]) {
        Ok(AcceptOutcome::Accepted { .. }) => {}
        Ok(other) => return Err(format!("reorg did not connect: {other:?}")),
        Err(e) => return Err(format!("reorg: {e}")),
    }
    push_setup(&mut out, &fork_a);
    push_setup(&mut out, &fork_b);
    let respend = mine_regtest_paying(
        fork_b.block_hash(),
        fork_b.header.time + REGTEST_BLOCK_SPACING,
        103,
        op_true(),
        vec![spend_tx(prev, value, Sequence::MAX)],
    );
    let fate = hub_fate(&session.hub, respend.clone())?;
    push_check(&mut out, &respend, fate);
    drop(session);
    Ok(out)
}

fn core_fate(reply: OracleReply) -> Result<Fate, String> {
    match reply {
        OracleReply::NullAccept => Ok(Fate::Accept),
        OracleReply::Reason(reason) => {
            if mutated_reason(&reason) {
                Ok(Fate::Mutated)
            } else {
                Ok(Fate::Reject)
            }
        }
        OracleReply::RpcError => Err("oracle rpc".into()),
        OracleReply::Dead => Err("oracle dead".into()),
    }
}

fn mutated_reason(reason: &str) -> bool {
    let r = reason.to_ascii_lowercase();
    r.contains("bad-txnmrklroot")
        || r.contains("bad-txns-duplicate")
        || r.contains("mutated")
        || r.contains("bad-witness-merkle-match")
        || r.contains("bad-witness-nonce-size")
        || r.contains("bad-seen-with-witness")
}

/// Submit the planned blocks. Agree when every compared fate matches Core.
pub fn compare_chain_plan(
    plan: &[PlannedSubmit],
    oracle: &dyn BlockOracle,
) -> Result<ChainReview, String> {
    if !oracle.liveness_ok() {
        return Err("oracle dead".into());
    }
    oracle.core_rewind_to_height(0)?;
    let mut saw = false;
    let mut all_accept = true;
    let mut mismatch: Option<(bool, bool)> = None;
    for step in plan {
        let reply = oracle.submitblock_hex(&step.hex);
        let core = core_fate(reply)?;
        let Some(ours) = step.fate else {
            continue;
        };
        saw = true;
        if ours != Fate::Accept {
            all_accept = false;
        }
        if mismatch.is_none() && ours != core {
            mismatch = Some((ours == Fate::Accept, core == Fate::Accept));
        }
    }
    if !saw {
        return Err("shape produced no comparison".into());
    }
    if let Some((ours, core)) = mismatch {
        Ok(ChainReview::Disagree { ours, core })
    } else {
        Ok(ChainReview::Agree { accept: all_accept })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct QueueOracle {
        replies: Vec<OracleReply>,
        n: Cell<usize>,
    }

    impl QueueOracle {
        fn new(replies: Vec<OracleReply>) -> Self {
            Self {
                replies,
                n: Cell::new(0),
            }
        }
    }

    impl BlockOracle for QueueOracle {
        fn submitblock_hex(&self, _hex: &str) -> OracleReply {
            let i = self.n.get();
            self.n.set(i + 1);
            self.replies
                .get(i)
                .cloned()
                .unwrap_or(OracleReply::RpcError)
        }
        fn liveness_ok(&self) -> bool {
            true
        }
        fn core_rewind_to_height(&self, _keep: u32) -> Result<(), &'static str> {
            Ok(())
        }
        fn core_reconsider_block(&self, _hash: &str) -> Result<(), &'static str> {
            Ok(())
        }
        fn core_invalidate_hash(&self, _hash: &str) -> Result<(), &'static str> {
            Ok(())
        }
        fn core_precious_block(&self, _hash: &str) -> Result<(), &'static str> {
            Ok(())
        }
    }

    fn reply_for(fate: Fate) -> OracleReply {
        match fate {
            Fate::Accept => OracleReply::NullAccept,
            Fate::Mutated => OracleReply::Reason("bad-txnmrklroot".into()),
            Fate::Reject => OracleReply::Reason("consensus".into()),
        }
    }

    fn flip(fate: Fate) -> Fate {
        match fate {
            Fate::Accept => Fate::Reject,
            Fate::Reject | Fate::Mutated => Fate::Accept,
        }
    }

    fn scripted(plan: &[PlannedSubmit], flip_checks: bool) -> Vec<OracleReply> {
        let mut flipped = false;
        plan.iter()
            .map(|step| match step.fate {
                None => OracleReply::NullAccept,
                Some(fate) => {
                    let use_fate = if flip_checks && !flipped {
                        flipped = true;
                        flip(fate)
                    } else {
                        fate
                    };
                    reply_for(use_fate)
                }
            })
            .collect()
    }

    #[test]
    fn each_chain_shape_classifies_against_the_hub() {
        for shape in 0..CHAIN_REVIEW_SHAPES {
            let plan = plan_chain_shape(shape).unwrap_or_else(|e| panic!("shape {shape}: {e}"));
            assert!(
                plan.iter().any(|s| s.fate.is_some()),
                "shape {shape} skipped the comparison"
            );
            let matched = compare_chain_plan(&plan, &QueueOracle::new(scripted(&plan, false)))
                .unwrap_or_else(|e| panic!("shape {shape}: {e}"));
            assert!(
                matches!(matched, ChainReview::Agree { .. }),
                "shape {shape} did not agree with its hub fate: {matched:?}"
            );
            let flipped = compare_chain_plan(&plan, &QueueOracle::new(scripted(&plan, true)))
                .unwrap_or_else(|e| panic!("shape {shape}: {e}"));
            assert!(
                matches!(flipped, ChainReview::Disagree { .. }),
                "shape {shape} did not disagree with the opposite reply: {flipped:?}"
            );
        }
    }
}
