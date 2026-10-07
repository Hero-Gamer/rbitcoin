//! Shared regtest chain builders so scenarios mine a mature chain once.

use std::path::Path;
use std::sync::OnceLock;

use bitcoin::hashes::Hash;
use bitcoin::{Amount, Block, BlockHash, Txid};
use rbitcoin_consensus::{
    accept_and_connect_block, pad_empty_from as consensus_pad, ChainParams, Milestone,
};
use rbitcoin_primitives::Height;
use rbitcoin_query::Query;

use crate::mine::{mine_regtest_block, regtest_genesis, spend_anyone_can_spend};

/// Blocks `0..=tip` accepted into `query`, including a spend of block-1 coinbase
/// at the tip when maturity allows.
#[derive(Clone)]
pub struct MatureRegtestChain {
    pub blocks: Vec<Block>,
    /// Height of the block that spends block-1 coinbase (last block).
    pub spend_height: u32,
    /// Coinbase txid of height-1 (the matured output we spend).
    pub matured_coinbase_txid: Txid,
}

impl MatureRegtestChain {
    pub fn tip_height(&self) -> u32 {
        (self.blocks.len() - 1) as u32
    }

    pub fn tip_hash(&self) -> BlockHash {
        self.blocks.last().unwrap().block_hash()
    }
}

/// Build genesis → pad through coinbase maturity → one spend of height-1 coinbase.
///
/// Mines into `query`. A plain-regtest empty store should use
/// [`open_mature_regtest_with_spend`], which mines once per process and copies.
pub fn build_mature_regtest_with_spend(query: &Query, params: &ChainParams) -> MatureRegtestChain {
    mine_mature_regtest_with_spend(query, params)
}

/// One plain-regtest mature chain per process. Each caller gets a private store copy.
struct SharedMatureRegtest {
    keep: crate::TempDir,
    chain: MatureRegtestChain,
}

fn shared_mature_regtest() -> &'static SharedMatureRegtest {
    static PAD: OnceLock<SharedMatureRegtest> = OnceLock::new();
    PAD.get_or_init(|| {
        let keep = crate::TempDir::labeled("mature-regtest-pad").expect("mature pad dir");
        let q = Query::open_or_create_tiny(keep.path()).unwrap();
        let chain = mine_mature_regtest_with_spend(&q, &ChainParams::regtest());
        q.flush().expect("flush mature regtest pad");
        drop(q);
        SharedMatureRegtest { keep, chain }
    })
}

fn plain_regtest(params: &ChainParams) -> bool {
    let base = ChainParams::regtest();
    params.network == base.network
        && params.genesis_hash == base.genesis_hash
        && params.pow_limit == base.pow_limit
        && params.checkpoints.is_empty()
        && params.signet_challenge.is_none()
        && params.bip34_hash.is_none()
        && params.csv_height() == base.csv_height()
        && params.segwit_height() == base.segwit_height()
        && params.subsidy_halving_interval() == base.subsidy_halving_interval()
        && params.no_pow_retargeting() == base.no_pow_retargeting()
        && params.allow_min_difficulty_blocks() == base.allow_min_difficulty_blocks()
        && params.difficulty_adjustment_interval() == base.difficulty_adjustment_interval()
}

fn dir_occupied(path: &Path) -> bool {
    std::fs::read_dir(path)
        .ok()
        .is_some_and(|mut d| d.next().is_some())
}

fn copy_store_tree(src: &Path, dst: &Path) {
    for ent in std::fs::read_dir(src).expect("read store") {
        let ent = ent.expect("store entry");
        let to = dst.join(ent.file_name());
        let ty = ent.file_type().expect("file type");
        if ty.is_dir() {
            std::fs::create_dir_all(&to).unwrap();
            copy_store_tree(&ent.path(), &to);
        } else {
            std::fs::copy(ent.path(), &to).unwrap();
        }
    }
}

/// Copy the process-lifetime plain-regtest mature chain into `store` and open it.
///
/// A non-empty directory, or params that are not plain regtest, is mined into
/// that store and does not read or write the shared pad. Header timestamps
/// stay the pad's.
pub fn open_mature_regtest_with_spend(
    store: impl AsRef<Path>,
    params: &ChainParams,
) -> (Query, MatureRegtestChain) {
    let store = store.as_ref();
    if !plain_regtest(params) || dir_occupied(store) {
        let q = Query::open_or_create_tiny(store).unwrap();
        let chain = mine_mature_regtest_with_spend(&q, params);
        return (q, chain);
    }
    let pad = shared_mature_regtest();
    std::fs::create_dir_all(store).unwrap();
    copy_store_tree(pad.keep.path(), store);
    let q = Query::open_or_create_tiny(store).unwrap();
    q.flush().expect("flush mature regtest copy");
    let tip = q.tip_height().map(|h| h.0);
    assert_eq!(tip, Some(pad.chain.tip_height()));
    let (_, rec) = q
        .header_at_height(Height(pad.chain.tip_height()))
        .unwrap()
        .unwrap();
    assert_eq!(rec.hash, *pad.chain.tip_hash().as_byte_array());
    (q, pad.chain.clone())
}

fn mine_mature_regtest_with_spend(query: &Query, params: &ChainParams) -> MatureRegtestChain {
    let ms = Milestone::NONE;
    let maturity = params.coinbase_maturity();

    let genesis = regtest_genesis();
    accept_and_connect_block(query, params, Height::GENESIS, &genesis, ms).unwrap();

    let mut blocks = vec![genesis.clone()];
    let mut tip = genesis.block_hash();
    let mut tip_time = genesis.header.time;

    let b1 = mine_regtest_block(tip, tip_time + 600, 1, vec![]);
    let matured_coinbase_txid = b1.txdata[0].compute_txid();
    accept_and_connect_block(query, params, Height(1), &b1, ms).unwrap();
    tip = b1.block_hash();
    tip_time = b1.header.time;
    blocks.push(b1);

    // Pad until height-1 has `maturity` confirmations → spendable at height `1 + maturity`.
    // After connecting height H, confs of h1 = H - 1. Need H - 1 >= maturity ⇒ H >= maturity + 1.
    let last_pad = maturity + 1; // inclusive; at this height spend is allowed for next block
    for h in 2..=last_pad {
        let b = mine_regtest_block(tip, tip_time + 600, h, vec![]);
        accept_and_connect_block(query, params, Height(h), &b, ms).unwrap();
        tip = b.block_hash();
        tip_time = b.header.time;
        blocks.push(b);
    }

    let spend_height = last_pad + 1;
    let spend = spend_anyone_can_spend(matured_coinbase_txid, 0, Amount::from_sat(49_0000_0000));
    let b_spend = mine_regtest_block(tip, tip_time + 600, spend_height, vec![spend]);
    accept_and_connect_block(query, params, Height(spend_height), &b_spend, ms).unwrap();
    blocks.push(b_spend);
    query.apply_sh_pending().unwrap();

    MatureRegtestChain {
        blocks,
        spend_height,
        matured_coinbase_txid,
    }
}

/// Fast empty pad via [`rbitcoin_consensus::pad_empty_from`].
///
/// Mines heights `from_h..=last` on top of `tip` / `tip_time`. Prefer this for
/// coinbase-maturity padding instead of looping `confirm_wire_run`.
///
/// Returns `(new_tip_hash, new_tip_time)`.
pub fn pad_empty_from(
    query: &Query,
    params: &ChainParams,
    tip: BlockHash,
    tip_time: u32,
    from_h: u32,
    last: u32,
) -> (BlockHash, u32) {
    let (tip, tip_time, _) = consensus_pad(query, params, tip, tip_time, from_h, last, 0);
    (tip, tip_time)
}

/// Assert reconstructed wire matches `original` at `height`.
pub fn assert_reconstruct_eq(query: &Query, height: u32, original: &Block) {
    use bitcoin::consensus::Encodable;

    let recon = query
        .reconstruct_block_at_height(Height(height))
        .unwrap_or_else(|e| panic!("reconstruct height {height}: {e}"));
    assert_eq!(
        recon.block_hash(),
        original.block_hash(),
        "hash height {height}"
    );
    assert_eq!(recon.header, original.header, "header height {height}");
    assert_eq!(recon.txdata.len(), original.txdata.len());
    for (i, (a, b)) in recon.txdata.iter().zip(original.txdata.iter()).enumerate() {
        let mut ra = Vec::new();
        let mut rb = Vec::new();
        a.consensus_encode(&mut ra).unwrap();
        b.consensus_encode(&mut rb).unwrap();
        assert_eq!(ra, rb, "tx wire height {height} index {i}");
    }
    let by_hash = query
        .reconstruct_block_by_hash(&original.block_hash().to_byte_array())
        .unwrap()
        .expect("by hash");
    assert_eq!(by_hash.block_hash(), original.block_hash());
    let via_ast = bitcoin::consensus::encode::serialize(&recon);
    let via_direct = query
        .witness_block_bytes_by_hash(&original.block_hash().to_byte_array())
        .unwrap()
        .expect("witness_block_bytes");
    assert_eq!(via_direct, via_ast, "witness_block_bytes height {height}");
}
