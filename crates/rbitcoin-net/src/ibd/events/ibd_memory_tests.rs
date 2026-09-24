//! Header-path and body-intake bounds (findings C03, C05).

use super::super::state::IbdWorkState;
use super::{apply_block_framed, on_headers_batch};
use bitcoin::absolute::LockTime;
use bitcoin::block::{Header, Version};
use bitcoin::consensus::encode::serialize;
use bitcoin::hashes::Hash;
use bitcoin::script::ScriptBuf;
use bitcoin::transaction::Version as TxVersion;
use bitcoin::BlockHash;
use bitcoin::{
    Amount, Block, CompactTarget, OutPoint, Sequence, Target, Transaction, TxIn, TxOut, Witness,
};
use std::sync::atomic::AtomicU32;

fn tmp_hub() -> (rbitcoin_query::testutil::TempDir, crate::chain::ChainHub) {
    crate::chain::tiny_regtest_hub_labeled("ibd-memory")
}

fn coinbase(height: u32) -> Transaction {
    let mut ss = if height == 0 {
        vec![0x00]
    } else {
        rbitcoin_consensus::bip34_height_script(height)
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
            value: Amount::from_sat(50_0000_0000),
            script_pubkey: ScriptBuf::from_bytes(vec![0x51]),
        }],
    }
}

fn mine(prev: BlockHash, time: u32, height: u32) -> Block {
    let bits = CompactTarget::from_consensus(0x207f_ffff);
    let mut block = Block {
        header: Header {
            version: Version::from_consensus(4),
            prev_blockhash: prev,
            merkle_root: bitcoin::TxMerkleNode::from_byte_array([0u8; 32]),
            time,
            bits,
            nonce: 0,
        },
        txdata: vec![coinbase(height)],
    };
    block.header.merkle_root = block.compute_merkle_root().unwrap();
    let target = Target::from_compact(bits);
    for nonce in 0..u32::MAX {
        block.header.nonce = nonce;
        if block.header.validate_pow(target).is_ok() {
            break;
        }
    }
    block
}

#[test]
fn rejected_header_batch_does_not_grow_path_or_explore() {
    let (dir, hub) = tmp_hub();
    hub.ensure_genesis().unwrap();
    let gen = hub.tip_hash().unwrap();
    let mut st = IbdWorkState::new(Vec::new(), hub.tip_hash(), hub.tip_height());
    let path_before = st.hash_height.len();
    let mut bad = mine(gen, 1_500_010_000, 1);
    bad.header.time = 0;
    on_headers_batch(&mut st, &hub, vec![bad.header]);
    assert_eq!(
        st.hash_height.len(),
        path_before,
        "invalid header must not enter the work path"
    );
    assert!(st.reorg.explore_need_hashes().is_empty());
    assert!(st.reorg.explore_tips().is_empty());

    let good = mine(gen, 1_500_010_000, 1);
    on_headers_batch(&mut st, &hub, vec![good.header]);
    assert!(
        st.hash_height.contains_key(&good.block_hash()),
        "a valid header still records path state"
    );

    let mut child = mine(good.block_hash(), 1_500_010_600, 2);
    child.header.time = 0;
    let broken = child.header.block_hash();
    on_headers_batch(&mut st, &hub, vec![good.header, child.header]);
    assert!(
        st.hash_height.contains_key(&good.block_hash()),
        "the valid prefix stays on the path"
    );
    assert!(
        !st.hash_height.contains_key(&broken),
        "header after a failed validation must not be noted"
    );

    // `good` was stored before this pair, so a search that returns nothing
    // still leaves it on the path. A fresh prefix has to be found by the
    // binary search or it never enters the work path.
    let fresh = mine(good.block_hash(), 1_500_011_200, 2);
    let mut bad_tail = mine(fresh.block_hash(), 1_500_011_800, 3);
    bad_tail.header.time = 0;
    let fresh_hash = fresh.block_hash();
    let tail_hash = bad_tail.header.block_hash();
    let added = on_headers_batch(&mut st, &hub, vec![fresh.header, bad_tail.header]);
    assert!(
        st.hash_height.contains_key(&fresh_hash),
        "binary search must keep the valid prefix of a rejected tail"
    );
    assert!(added >= 1, "the valid prefix is enqueued");
    assert!(
        !st.hash_height.contains_key(&tail_hash),
        "rejected tail must not be noted"
    );

    let second = mine(fresh.block_hash(), 1_500_012_200, 3);
    let mut bad_third = mine(second.block_hash(), 1_500_012_800, 4);
    bad_third.header.time = 0;
    let second_hash = second.block_hash();
    on_headers_batch(
        &mut st,
        &hub,
        vec![fresh.header, second.header, bad_third.header],
    );
    assert!(
        st.hash_height.contains_key(&second_hash),
        "both headers before a rejected tail stay on the path"
    );
    assert!(!st.hash_height.contains_key(&bad_third.header.block_hash()));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn unsolicited_body_is_not_copied_into_the_queue() {
    let (dir, hub) = tmp_hub();
    hub.ensure_genesis().unwrap();
    let gen = hub.tip_hash().unwrap();
    let block = mine(gen, 1_500_020_000, 1);
    let hash = block.block_hash();
    let payload = serialize(&block);
    let mut st = IbdWorkState::new(Vec::new(), hub.tip_hash(), hub.tip_height());
    st.record_height(hash, 1);
    st.header_fks.insert(hash, rbitcoin_primitives::Fk(1));
    let before = hub.query.block_queue_stats().1;
    let write_next = AtomicU32::new(0);
    apply_block_framed(&mut st, &hub, &write_next, None, 0, hash, payload.clone());
    assert_eq!(
        hub.query.block_queue_stats().1,
        before,
        "unsolicited body must not be copied into the queue"
    );

    st.inflight
        .insert(hash, super::super::state::InflightReq::new(0));
    apply_block_framed(&mut st, &hub, &write_next, None, 0, hash, payload);
    assert!(
        hub.query.block_queue_stats().1 > before,
        "a requested body is still queued"
    );
    let queued = hub.query.block_queue_stats().1;
    st.inflight
        .insert(hash, super::super::state::InflightReq::new(0));
    apply_block_framed(&mut st, &hub, &write_next, None, 0, hash, serialize(&block));
    assert_eq!(
        hub.query.block_queue_stats().1,
        queued,
        "a second copy of a queued hash is dropped"
    );
    let _ = std::fs::remove_dir_all(dir);
}
