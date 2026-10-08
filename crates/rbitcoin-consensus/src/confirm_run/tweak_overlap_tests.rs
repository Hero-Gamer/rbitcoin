//! Script and tweak jobs of one batch share a steal wave.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use bitcoin::absolute::LockTime;
use bitcoin::block::{Header, Version};
use bitcoin::hashes::{hash160, Hash};
use bitcoin::script::ScriptBuf;
use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
use bitcoin::transaction::Version as TxVersion;
use bitcoin::{
    Amount, Block, BlockHash, CompactTarget, OutPoint, Sequence, Transaction, TxIn, TxMerkleNode,
    TxOut, Witness,
};
use rbitcoin_primitives::{Fk, Height};

use super::index::tweak_gate;
use super::scripts::drive_script_waves_with;
use super::{LoadedBatch, Prepared};
use crate::script_pool::steal_test_gate;
use crate::silent_payments::tweak_from_tx;

fn sp_scripts() -> (Vec<u8>, ScriptBuf, ScriptBuf) {
    let secp = Secp256k1::new();
    let sk = SecretKey::from_slice(&[2u8; 32]).expect("key");
    let pk = PublicKey::from_secret_key(&secp, &sk);
    let ser = pk.serialize().to_vec();
    let h160 = hash160::Hash::hash(&ser);
    let mut p2wpkh = vec![0x00, 0x14];
    p2wpkh.extend_from_slice(h160.as_byte_array());
    let (xonly, _) = pk.x_only_public_key();
    let mut p2tr = vec![0x51, 0x20];
    p2tr.extend_from_slice(&xonly.serialize());
    (
        ser,
        ScriptBuf::from_bytes(p2wpkh),
        ScriptBuf::from_bytes(p2tr),
    )
}

fn coinbase(spk: ScriptBuf) -> Transaction {
    Transaction {
        version: TxVersion::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::from_bytes(vec![0x01, 0x01]),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(50_0000_0000),
            script_pubkey: spk,
        }],
    }
}

fn spend(prev: bitcoin::Txid, spk: ScriptBuf, witness: Witness) -> Transaction {
    Transaction {
        version: TxVersion::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint {
                txid: prev,
                vout: 0,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness,
        }],
        output: vec![TxOut {
            value: Amount::from_sat(49_0000_0000),
            script_pubkey: spk,
        }],
    }
}

fn bare_block(txdata: Vec<Transaction>) -> Block {
    Block {
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

fn prepared_block(
    height: u32,
    block: &Block,
    tx_fks: Vec<Fk>,
    spends: Vec<([u8; 32], u32, Fk, Fk, u32)>,
) -> Prepared {
    Prepared {
        height: Height(height),
        header_fk: Fk(7),
        tx_fks,
        jobs: Vec::new(),
        spends,
        fees: 0,
        check_scripts: false,
        time: block.header.time,
        bits: block.header.bits,
        hash: [height as u8; 32],
        prev_mtp: 0,
    }
}

fn loaded_tweak_batch(block: Block, prep: Prepared) -> LoadedBatch {
    LoadedBatch {
        prepared: vec![prep],
        wire_blocks: vec![Arc::new(block)],
        batch_parents: rbitcoin_query::BatchParents::new(),
        script_preverified: super::ScriptPreverified::new(),
        archive_plan: None,
        index_want: super::index::IndexWant {
            filters: false,
            tweak_origin: Some(0),
        },
        stats: Arc::new(rbitcoin_query::ConfirmStats::default()),
    }
}

fn empty_batch() -> LoadedBatch {
    LoadedBatch {
        prepared: Vec::new(),
        wire_blocks: Vec::new(),
        batch_parents: rbitcoin_query::BatchParents::new(),
        script_preverified: super::ScriptPreverified::new(),
        archive_plan: None,
        index_want: super::index::IndexWant::default(),
        stats: Arc::new(rbitcoin_query::ConfirmStats::default()),
    }
}

/// One eligible same-block spend. A single tweak job must still run on
/// `rbtc-scripts-*` (the one-item inline path would run it on `ibd-confirm`).
fn one_eligible_spend() -> (Block, Prepared) {
    let (ser, p2wpkh, p2tr) = sp_scripts();
    let cb = coinbase(p2wpkh);
    let tx1 = spend(
        cb.compute_txid(),
        p2tr,
        Witness::from_slice(&[vec![0u8; 64].as_slice(), ser.as_slice()]),
    );
    let cb_txid = cb.compute_txid().to_byte_array();
    let block = bare_block(vec![cb, tx1]);
    let prep = prepared_block(
        5,
        &block,
        vec![Fk(1), Fk(2)],
        vec![(cb_txid, 0, Fk(2), Fk::NULL, 0)],
    );
    (block, prep)
}

/// Coinbase, a P2WPKH→P2TR spend, and a same-block child of that spend.
fn same_block_chain() -> (Block, Prepared, Vec<Option<[u8; 33]>>) {
    let (ser, p2wpkh, p2tr) = sp_scripts();
    let cb = coinbase(p2wpkh);
    let tx1 = spend(
        cb.compute_txid(),
        p2tr.clone(),
        Witness::from_slice(&[vec![0u8; 64].as_slice(), ser.as_slice()]),
    );
    let tx2 = spend(tx1.compute_txid(), p2tr, Witness::new());
    let expect = vec![
        None,
        tweak_from_tx(&tx1, &[cb.output[0].clone()]).map(|t| t.tweak),
        tweak_from_tx(&tx2, &[tx1.output[0].clone()]).map(|t| t.tweak),
    ];
    let cb_txid = cb.compute_txid().to_byte_array();
    let tx1_txid = tx1.compute_txid().to_byte_array();
    let block = bare_block(vec![cb, tx1, tx2]);
    let prep = prepared_block(
        9,
        &block,
        vec![Fk(1), Fk(2), Fk(3)],
        vec![
            (cb_txid, 0, Fk(2), Fk::NULL, 0),
            (tx1_txid, 0, Fk(3), Fk::NULL, 0),
        ],
    );
    (block, prep, expect)
}

/// The next batch is taken while this batch's tweak job is still blocked.
/// The outcome waits for that job. Heights stay in order.
#[test]
fn tweak_job_overlaps_the_next_batch() {
    let _gate = steal_test_gate();
    tweak_gate::arm();
    let (block, prep) = one_eligible_spend();
    let (tx, rx) = mpsc::sync_channel(4);
    let takes = Arc::new(AtomicUsize::new(0));
    let outcomes = Arc::new(Mutex::new(Vec::new()));
    let takes_w = Arc::clone(&takes);
    let outcomes_w = Arc::clone(&outcomes);
    let stage = thread::Builder::new()
        .name("ibd-confirm".into())
        .spawn(move || {
            drive_script_waves_with(
                &rx,
                |_batch, _wait| {
                    takes_w.fetch_add(1, Ordering::SeqCst);
                },
                |_ok, meta| {
                    outcomes_w.lock().unwrap().push(meta.first_h);
                    true
                },
                |_e, _meta, _dropped| false,
                || false,
            );
        })
        .expect("spawn publisher");
    tx.send((loaded_tweak_batch(block, prep), 0))
        .expect("batch 1");
    tx.send((empty_batch(), 0)).expect("batch 2");
    drop(tx);
    crate::unpark_script_publisher();

    let deadline = Instant::now() + Duration::from_secs(5);
    while tweak_gate::entered() == 0 {
        if Instant::now() > deadline {
            tweak_gate::release();
            let _ = stage.join();
            panic!("tweak job never entered the gate");
        }
        thread::sleep(Duration::from_millis(10));
    }
    thread::sleep(Duration::from_millis(150));
    let takes_while_blocked = takes.load(Ordering::SeqCst);
    let outcomes_while_blocked = outcomes.lock().unwrap().clone();
    let on_worker = tweak_gate::on_script_worker();
    tweak_gate::release();
    stage.join().expect("publisher");
    let heights = outcomes.lock().unwrap().clone();

    assert!(
        takes_while_blocked >= 2,
        "second batch must be taken while the tweak job is blocked, takes={takes_while_blocked}"
    );
    assert!(
        outcomes_while_blocked.is_empty(),
        "first outcome must wait for the tweak job, got {outcomes_while_blocked:?}"
    );
    assert!(
        on_worker,
        "tweak job must run on rbtc-scripts-*, not ibd-confirm"
    );
    assert_eq!(heights, vec![5, 0]);
}

#[test]
fn same_block_spend_matches_tweak_from_tx() {
    let _gate = steal_test_gate();
    let (block, prep, expect) = same_block_chain();
    assert!(expect[1].is_some(), "parent spend is eligible");
    assert!(expect[2].is_some(), "same-block child is eligible");
    let batch = loaded_tweak_batch(block, prep);
    let ok = super::confirm_scripts_phase(batch).expect("scripts");
    let tweaks = &ok.batch.index_seal.tweaks;
    assert_eq!(tweaks.len(), 1);
    assert_eq!(tweaks[0].0, Height(9));
    assert_eq!(tweaks[0].1, Fk(7));
    assert_eq!(tweaks[0].2, expect);
}
