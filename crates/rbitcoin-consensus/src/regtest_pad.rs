//! Fast regtest empty-block pad (maturity) for tests in dependent crates.
//!
//! Prefer this over a local `for h in 1..=100 { mine_pow; connect }` loop.

use bitcoin::absolute::LockTime;
use bitcoin::block::{Header, Version};
use bitcoin::hashes::Hash;
use bitcoin::transaction::Version as TxVersion;
use bitcoin::{
    Amount, Block, BlockHash, CompactTarget, OutPoint, ScriptBuf, Sequence, Target, Transaction,
    TxIn, TxMerkleNode, TxOut, Txid, Witness,
};
use rbitcoin_primitives::Height;
use rbitcoin_query::Query;

use crate::block::apply_witness_commitment;
use crate::{
    accept_and_connect_block, bip34_height_script, block_has_witness, block_subsidy, ChainParams,
    Milestone,
};

pub const REGTEST_POW_BITS: u32 = 0x207f_ffff;
pub const REGTEST_BLOCK_SPACING: u32 = 600;

/// Grind `header.nonce` until PoW matches `header.bits`. Does not change version.
///
/// Regtest bits (`0x207fffff`) hit in the first few nonces. An `OP_TRUE`
/// signet keeps genesis bits (`0x1e0377ae`), about 5e6 hashes. Past the
/// trivial prefix the search reuses the SHA256 midstate of the first 64
/// header bytes. Off the `tip-accept` thread that search uses up to four
/// cores; on `tip-accept` it stays on one core so the lane cannot join
/// workers that wait for the lane.
pub fn grind_regtest_pow(header: &mut Header) {
    let target = Target::from_compact(header.bits);
    const QUICK: u32 = 4096;
    for nonce in 0..QUICK {
        header.nonce = nonce;
        if header.validate_pow(target).is_ok() {
            return;
        }
    }
    let nonce = grind_header_midstate(header, target, QUICK);
    header.nonce = nonce;
    debug_assert!(header.validate_pow(target).is_ok());
}

fn on_tip_accept_thread() -> bool {
    std::thread::current().name() == Some("tip-accept")
}

/// SHA256 midstate after the first 64 header bytes. Nonce is the last 4
/// bytes. No heap per nonce.
fn grind_header_midstate(header: &Header, target: Target, start: u32) -> u32 {
    use bitcoin::consensus::Encodable;

    let mut raw = Vec::with_capacity(80);
    header
        .consensus_encode(&mut raw)
        .expect("header encode into vec");
    debug_assert_eq!(raw.len(), 80);
    let workers = if on_tip_accept_thread() {
        1
    } else {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .clamp(1, 4)
    };
    if workers == 1 {
        return grind_header_midstate_one(&raw, target, start, 1, None)
            .expect("regtest pow grind exhausted");
    }
    let stop = std::sync::atomic::AtomicBool::new(false);
    let hit = std::sync::atomic::AtomicBool::new(false);
    let found = std::sync::atomic::AtomicU32::new(u32::MAX);
    std::thread::scope(|scope| {
        for worker in 0..workers {
            let raw = &raw;
            let stop = &stop;
            let hit = &hit;
            let found = &found;
            let begin = start.saturating_add(worker as u32);
            scope.spawn(move || {
                if let Some(nonce) =
                    grind_header_midstate_one(raw, target, begin, workers as u32, Some(stop))
                {
                    found.fetch_min(nonce, std::sync::atomic::Ordering::Relaxed);
                    hit.store(true, std::sync::atomic::Ordering::Relaxed);
                    stop.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            });
        }
    });
    if !hit.load(std::sync::atomic::Ordering::Relaxed) {
        panic!("regtest pow grind exhausted");
    }
    found.load(std::sync::atomic::Ordering::Relaxed)
}

/// Search `start, start+step, …`. `step == 1` is the single-core scan.
///
/// `stop` lets sibling cores abandon the search once any stride has a hit.
/// Any nonce that meets `target` is a valid block; the caller keeps the
/// smallest one that was stored.
fn grind_header_midstate_one(
    raw: &[u8],
    target: Target,
    start: u32,
    step: u32,
    stop: Option<&std::sync::atomic::AtomicBool>,
) -> Option<u32> {
    use bitcoin::hashes::{sha256, Hash, HashEngine};

    debug_assert!(step >= 1);
    let mut engine = sha256::Hash::engine();
    engine.input(&raw[..64]);
    let mid = engine.midstate();
    let mut tail = [0u8; 16];
    tail.copy_from_slice(&raw[64..80]);
    let mut nonce = start;
    let mut spins: u32 = 0;
    loop {
        if spins == 0 && stop.is_some_and(|s| s.load(std::sync::atomic::Ordering::Relaxed)) {
            return None;
        }
        spins = spins.wrapping_add(1) % 1024;
        tail[12..16].copy_from_slice(&nonce.to_le_bytes());
        let mut second = sha256::HashEngine::from_midstate(mid, 64);
        second.input(&tail);
        let first = sha256::Hash::from_engine(second);
        let pow = sha256::Hash::hash(first.as_byte_array());
        let hash = bitcoin::BlockHash::from_byte_array(pow.to_byte_array());
        if target.is_met_by(hash) {
            return Some(nonce);
        }
        nonce = nonce.checked_add(step)?;
    }
}

/// Fuzzer / tests keep txdata + version. Harness owns prev/bits/time/merkle/nonce.
pub fn prepare_regtest_candidate(block: &mut Block, prev: BlockHash, time: u32) {
    block.header.prev_blockhash = prev;
    block.header.bits = CompactTarget::from_consensus(REGTEST_POW_BITS);
    block.header.time = time;
    block.header.merkle_root = block
        .compute_merkle_root()
        .unwrap_or(TxMerkleNode::from_byte_array([0u8; 32]));
    grind_regtest_pow(&mut block.header);
}

/// Mine one empty-ish regtest block (trivial bits).
pub fn mine_empty_regtest(prev: BlockHash, time: u32, height: u32) -> Block {
    mine_regtest_paying(
        prev,
        time,
        height,
        ScriptBuf::from_bytes(vec![0x51]),
        Vec::new(),
    )
}

/// Mine one regtest block paying `script_pubkey`, optional extra txs, trivial bits.
///
/// Adds a BIP141 witness commitment when any input carries witness data.
/// Confirm still goes through the normal accept/connect path (caller).
pub fn mine_regtest_paying(
    prev: BlockHash,
    time: u32,
    height: u32,
    script_pubkey: ScriptBuf,
    extra_txs: Vec<Transaction>,
) -> Block {
    let bits = CompactTarget::from_consensus(REGTEST_POW_BITS);
    // Post-BIP65 (regtest height 1) requires nVersion ≥ 4. Core generate uses
    // VERSIONBITS_TOP_BITS; 4 is the buried minimum and enough for dersig/cltv.
    let header = Header {
        version: Version::from_consensus(4),
        prev_blockhash: prev,
        merkle_root: TxMerkleNode::from_byte_array([0u8; 32]),
        time,
        bits,
        nonce: 0,
    };
    let mut txdata = Vec::with_capacity(1 + extra_txs.len());
    txdata.push(coinbase_paying(height, script_pubkey));
    txdata.extend(extra_txs);
    let mut block = Block { header, txdata };
    if block_has_witness(&block) {
        apply_witness_commitment(&mut block);
    } else if let Some(root) = block.compute_merkle_root() {
        block.header.merkle_root = root;
    }
    grind_regtest_pow(&mut block.header);
    block
}

/// One signet block whose challenge is `OP_TRUE`.
///
/// Bits come from the caller (the parent header at height 1). The coinbase
/// witness commitment carries an empty scriptSig / empty witness solution,
/// which is the only solution `OP_TRUE` accepts. PoW is a nonce grind against
/// signet's genesis target (`0x1e0377ae`, about 5e6 hashes, not regtest's
/// `0x207fffff`).
pub fn mine_op_true_signet_paying(
    params: &ChainParams,
    prev: BlockHash,
    time: u32,
    height: u32,
    bits: CompactTarget,
    script_pubkey: ScriptBuf,
    extra_txs: Vec<Transaction>,
) -> Block {
    let header = Header {
        version: Version::from_consensus(4),
        prev_blockhash: prev,
        merkle_root: TxMerkleNode::from_byte_array([0u8; 32]),
        time,
        bits,
        nonce: 0,
    };
    let mut txdata = Vec::with_capacity(1 + extra_txs.len());
    txdata.push(coinbase_paying_params(height, script_pubkey, params));
    txdata.extend(extra_txs);
    let mut block = Block { header, txdata };
    block.txdata[0].input[0].witness = Witness::from_slice(&[vec![0u8; 32]]);
    apply_witness_commitment(&mut block);
    append_op_true_signet_push(&mut block);
    grind_regtest_pow(&mut block.header);
    block
}

/// BIP325: second push in the witness-commitment script is header `ecc7daa2`
/// plus an empty scriptSig and an empty witness (`0x00 0x00`).
fn append_op_true_signet_push(block: &mut Block) {
    const SIGNET_HEADER: [u8; 4] = [0xec, 0xc7, 0xda, 0xa2];
    let mut payload = Vec::with_capacity(6);
    payload.extend_from_slice(&SIGNET_HEADER);
    payload.extend_from_slice(&[0x00, 0x00]);
    let spk = &mut block.txdata[0]
        .output
        .last_mut()
        .expect("witness commitment output")
        .script_pubkey;
    let mut bytes = spk.as_bytes().to_vec();
    bytes.push(u8::try_from(payload.len()).expect("signet solution push"));
    bytes.extend_from_slice(&payload);
    *spk = ScriptBuf::from_bytes(bytes);
    if let Some(root) = block.compute_merkle_root() {
        block.header.merkle_root = root;
    }
}

fn coinbase_paying(height: u32, script_pubkey: ScriptBuf) -> Transaction {
    coinbase_paying_params(height, script_pubkey, &ChainParams::regtest())
}

fn coinbase_paying_params(
    height: u32,
    script_pubkey: ScriptBuf,
    params: &ChainParams,
) -> Transaction {
    let mut ss = if height == 0 {
        vec![0x00]
    } else {
        bip34_height_script(height)
    };
    while ss.len() < 2 {
        ss.push(0x00);
    }
    Transaction {
        version: TxVersion::ONE,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::from_bytes(ss),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(block_subsidy(height, params) as u64),
            script_pubkey,
        }],
    }
}

/// Connect empty blocks `from_h..=last`. Returns tip hash/time and coinbase txids
/// for heights `1..=collect_coinbases` (empty if `collect_coinbases == 0`).
pub fn pad_empty_from(
    query: &Query,
    params: &ChainParams,
    mut tip: BlockHash,
    mut tip_time: u32,
    from_h: u32,
    last: u32,
    collect_coinbases: u32,
) -> (BlockHash, u32, Vec<Txid>) {
    let ms = Milestone::NONE;
    let mut cbs = Vec::new();
    for h in from_h..=last {
        let b = mine_empty_regtest(tip, tip_time + 600, h);
        if h >= 1 && h <= collect_coinbases {
            cbs.push(b.txdata[0].compute_txid());
        }
        accept_and_connect_block(query, params, Height(h), &b, ms).unwrap();
        tip = b.block_hash();
        tip_time = b.header.time;
    }
    query.apply_sh_pending().unwrap();
    (tip, tip_time, cbs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn pad_collects_early_coinbases() {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("rbtc-pad-{n}"));
        let q = Query::open_or_create_tiny(&dir).unwrap();
        let params = ChainParams::regtest();
        let genesis = bitcoin::blockdata::constants::genesis_block(bitcoin::Network::Regtest);
        accept_and_connect_block(&q, &params, Height::GENESIS, &genesis, Milestone::NONE).unwrap();
        let (_tip, _t, cbs) = pad_empty_from(
            &q,
            &params,
            genesis.block_hash(),
            genesis.header.time,
            1,
            3,
            2,
        );
        assert_eq!(cbs.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mine_regtest_paying_sets_coinbase_script() {
        let genesis = bitcoin::blockdata::constants::genesis_block(bitcoin::Network::Regtest);
        let script =
            ScriptBuf::from_bytes(vec![0x00, 0x14].into_iter().chain([0x11u8; 20]).collect());
        let b = mine_regtest_paying(
            genesis.block_hash(),
            genesis.header.time + 1,
            1,
            script.clone(),
            vec![],
        );
        assert_eq!(b.txdata[0].output[0].script_pubkey, script);
        assert_eq!(b.header.prev_blockhash, genesis.block_hash());
        let target = Target::from_compact(b.header.bits);
        assert!(b.header.validate_pow(target).is_ok());
    }

    #[test]
    fn op_true_signet_block_meets_pow_and_challenge() {
        use bitcoin::script::Script;
        let params =
            ChainParams::custom_signet(ScriptBuf::from_bytes(vec![0x51]), 10 * 60).unwrap();
        let genesis = bitcoin::blockdata::constants::genesis_block(bitcoin::Network::Signet);
        let block = mine_op_true_signet_paying(
            &params,
            genesis.block_hash(),
            genesis.header.time.saturating_add(1),
            1,
            genesis.header.bits,
            ScriptBuf::from_bytes(vec![0x51]),
            vec![],
        );
        let target = Target::from_compact(block.header.bits);
        assert!(block.header.validate_pow(target).is_ok());
        crate::signet::validate_signet_block_solution(&block, Script::from_bytes(&[0x51])).unwrap();
    }

    #[test]
    fn mine_empty_regtest_uses_regtest_halving_subsidy() {
        let genesis = bitcoin::blockdata::constants::genesis_block(bitcoin::Network::Regtest);
        let prev = genesis.block_hash();
        let t = genesis.header.time + 1;
        let b149 = mine_empty_regtest(prev, t, 149);
        let b150 = mine_empty_regtest(prev, t, 150);
        assert_eq!(
            b149.txdata[0].output[0].value,
            Amount::from_sat(50_0000_0000)
        );
        assert_eq!(
            b150.txdata[0].output[0].value,
            Amount::from_sat(25_0000_0000)
        );
    }

    #[test]
    fn grind_regtest_pow_changes_nonce_only() {
        let genesis = bitcoin::blockdata::constants::genesis_block(bitcoin::Network::Regtest);
        let mut header = genesis.header;
        header.bits = CompactTarget::from_consensus(REGTEST_POW_BITS);
        header.nonce = 0;
        let version = header.version;
        let prev = header.prev_blockhash;
        let merkle = header.merkle_root;
        let time = header.time;
        let bits = header.bits;
        grind_regtest_pow(&mut header);
        assert_eq!(header.version, version);
        assert_eq!(header.prev_blockhash, prev);
        assert_eq!(header.merkle_root, merkle);
        assert_eq!(header.time, time);
        assert_eq!(header.bits, bits);
        let target = Target::from_compact(header.bits);
        assert!(header.validate_pow(target).is_ok());
    }

    #[test]
    fn prepare_regtest_candidate_keeps_version_and_txids() {
        let genesis = bitcoin::blockdata::constants::genesis_block(bitcoin::Network::Regtest);
        let mut block = mine_empty_regtest(genesis.block_hash(), genesis.header.time + 1, 1);
        block.header.version = Version::from_consensus(5);
        let txids: Vec<_> = block.txdata.iter().map(|t| t.compute_txid()).collect();
        let version = block.header.version;
        prepare_regtest_candidate(
            &mut block,
            genesis.block_hash(),
            genesis.header.time + REGTEST_BLOCK_SPACING,
        );
        assert_eq!(block.header.version, version);
        let after: Vec<_> = block.txdata.iter().map(|t| t.compute_txid()).collect();
        assert_eq!(after, txids);
        assert_eq!(block.header.prev_blockhash, genesis.block_hash());
        assert_eq!(
            block.header.bits,
            CompactTarget::from_consensus(REGTEST_POW_BITS)
        );
        assert_eq!(
            block.header.merkle_root,
            block.compute_merkle_root().unwrap()
        );
        let target = Target::from_compact(block.header.bits);
        assert!(block.header.validate_pow(target).is_ok());
    }

    #[test]
    fn regtest_height1_fixture_matches_mine_empty() {
        let genesis = bitcoin::blockdata::constants::genesis_block(bitcoin::Network::Regtest);
        let b = mine_empty_regtest(
            genesis.block_hash(),
            genesis.header.time + REGTEST_BLOCK_SPACING,
            1,
        );
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/regtest_height1.bin");
        let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let got: Block = bitcoin::consensus::encode::deserialize(&raw).expect("fixture block");
        assert_eq!(got.header.prev_blockhash, genesis.block_hash());
        assert_eq!(
            got.header.bits,
            CompactTarget::from_consensus(REGTEST_POW_BITS)
        );
        let target = Target::from_compact(got.header.bits);
        assert!(got.header.validate_pow(target).is_ok());
        assert_eq!(
            bitcoin::consensus::encode::serialize(&b),
            raw,
            "regtest_height1.bin must match mine_empty_regtest(genesis, t+600, 1)"
        );
    }
}
