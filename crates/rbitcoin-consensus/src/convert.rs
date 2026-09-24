use crate::error::ConsensusError;
use bitcoin::block::Header;
use bitcoin::hashes::Hash;
use bitcoin::Transaction;
use rbitcoin_primitives::Fk;
use rbitcoin_query::{Query, TxApply};
use rbitcoin_store::{HeaderRecord, InputRecord, OutputRecord, TxRecord};

/// Header row from wire fields. `hash` is the caller-computed `header.block_hash()`.
pub fn header_to_record(prev_fk: Fk, header: &Header, hash: [u8; 32]) -> HeaderRecord {
    HeaderRecord {
        prev_fk,
        version: header.version.to_consensus(),
        timestamp: header.time,
        bits: header.bits.to_consensus(),
        nonce: header.nonce,
        merkle_root: header.merkle_root.to_byte_array(),
        hash,
        size: 0,
        weight: 0,
    }
}

pub fn block_to_apply(
    query: &Query,
    header: &Header,
    txs: &[Transaction],
) -> Result<(HeaderRecord, Vec<TxApply>), ConsensusError> {
    let txids: Vec<[u8; 32]> = txs
        .iter()
        .map(|t| t.compute_txid().to_byte_array())
        .collect();
    block_to_apply_with_txids(query, header, txs, &txids)
}

/// Build archive records using **precomputed** txids (from structure validation).
///
/// Avoids a second `compute_txid` pass over every transaction — the hot path for
/// multi-worker IBD prep on large blocks.
pub fn block_to_apply_with_txids(
    query: &Query,
    header: &Header,
    txs: &[Transaction],
    txids: &[[u8; 32]],
) -> Result<(HeaderRecord, Vec<TxApply>), ConsensusError> {
    if txs.len() != txids.len() {
        return Err(ConsensusError::BadBlock("txid count mismatch"));
    }
    let prev_fk = if header.prev_blockhash.to_byte_array() == [0u8; 32] {
        Fk::NULL
    } else {
        query
            .get_header_by_hash(header.prev_blockhash.as_byte_array())?
            .map(|(fk, _)| fk)
            .ok_or(ConsensusError::BadPrev)?
    };
    block_to_apply_with_txids_prev(prev_fk, header, txs, txids)
}

/// Like [`block_to_apply_with_txids`] but **no store access** — caller supplies
/// `prev_fk` (use [`Fk::NULL`] on the IBD path where the header row already exists).
pub fn block_to_apply_with_txids_prev(
    prev_fk: Fk,
    header: &Header,
    txs: &[Transaction],
    txids: &[[u8; 32]],
) -> Result<(HeaderRecord, Vec<TxApply>), ConsensusError> {
    if txs.len() != txids.len() {
        return Err(ConsensusError::BadBlock("txid count mismatch"));
    }
    let hash = header.block_hash().to_byte_array();
    let header_rec = header_to_record(prev_fk, header, hash);
    let mut out = Vec::with_capacity(txs.len());
    for (tx, txid) in txs.iter().zip(txids.iter()) {
        out.push(tx_to_apply(tx, *txid)?);
    }
    Ok((header_rec, out))
}

fn tx_to_apply(tx: &Transaction, txid: [u8; 32]) -> Result<TxApply, ConsensusError> {
    let inputs: Vec<InputRecord> = tx
        .input
        .iter()
        .map(|inp| {
            let is_cb = inp.previous_output.is_null()
                || (inp.previous_output.txid.to_byte_array() == [0u8; 32]
                    && inp.previous_output.vout == u32::MAX);
            InputRecord {
                prev_txid: inp.previous_output.txid.to_byte_array(),
                // Archive resolve fills create_fk before pack; coinbase stays NULL.
                create_fk: Fk::NULL,
                prev_index: if is_cb {
                    u32::MAX
                } else {
                    inp.previous_output.vout
                },
                sequence: inp.sequence.to_consensus_u32(),
                script_sig: inp.script_sig.to_bytes(),
                witness: inp.witness.to_vec(),
            }
        })
        .collect();

    let outputs: Vec<OutputRecord> = tx
        .output
        .iter()
        .map(|o| OutputRecord::unspent(o.value.to_sat() as i64, o.script_pubkey.to_bytes()))
        .collect();

    Ok(TxApply {
        tx: TxRecord {
            txid,
            version: tx.version.0,
            locktime: tx.lock_time.to_consensus_u32(),
            input_start_fk: Fk::NULL,
            input_count: inputs.len() as u32,
            output_start_fk: Fk::NULL,
            output_count: outputs.len() as u32,
        },
        inputs,
        outputs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::absolute::LockTime;
    use bitcoin::block::{Header, Version};
    use bitcoin::transaction::Version as TxVersion;
    use bitcoin::{
        Amount, BlockHash, CompactTarget, OutPoint, ScriptBuf, Sequence, Transaction, TxIn,
        TxMerkleNode, TxOut, Witness,
    };

    #[test]
    fn header_to_record_stores_caller_hash() {
        let header = Header {
            version: Version::ONE,
            prev_blockhash: BlockHash::from_byte_array([0; 32]),
            merkle_root: TxMerkleNode::from_byte_array([0; 32]),
            time: 1,
            bits: CompactTarget::from_consensus(0x207f_ffff),
            nonce: 0,
        };
        let computed = header.block_hash().to_byte_array();
        let rec = header_to_record(Fk(3), &header, computed);
        assert_eq!(rec.hash, computed);
        assert_eq!(rec.prev_fk, Fk(3));
        assert_eq!(rec.timestamp, 1);
        let distinct = [0xab; 32];
        assert_ne!(distinct, computed);
        let rec2 = header_to_record(Fk::NULL, &header, distinct);
        assert_eq!(rec2.hash, distinct);
    }

    #[test]
    fn apply_with_precomputed_txid_matches_fresh_hash() {
        let tx = Transaction {
            version: TxVersion::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::from_bytes(vec![0x01]),
                sequence: Sequence::MAX,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(50_0000_0000),
                script_pubkey: ScriptBuf::from_bytes(vec![0x51]),
            }],
        };
        let want = tx.compute_txid().to_byte_array();
        let apply = tx_to_apply(&tx, want).unwrap();
        assert_eq!(apply.tx.txid, want);
        assert_eq!(apply.inputs.len(), 1);
        assert_eq!(apply.outputs[0].value, 50_0000_0000);
        let edges = [rbitcoin_query::SpendEdge {
            prev_txid: [0u8; 32],
            vout: u32::MAX,
            spend_fk: Fk(9),
            create_fk: Fk::NULL,
            vin: 0,
        }];
        let ins = rbitcoin_query::input_records_from_wire(&tx, Fk(9), &edges).unwrap();
        assert_eq!(ins, apply.inputs);
    }

    #[test]
    fn txid_count_mismatch_and_prev_null_genesis() {
        use bitcoin::block::{Header, Version};
        use bitcoin::{BlockHash, CompactTarget, TxMerkleNode};

        let tx = Transaction {
            version: TxVersion::ONE,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::from_bytes(vec![0x00, 0x01]),
                sequence: Sequence::MAX,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(50),
                script_pubkey: ScriptBuf::from_bytes(vec![0x51]),
            }],
        };
        let header = Header {
            version: Version::ONE,
            prev_blockhash: BlockHash::from_byte_array([0; 32]),
            merkle_root: TxMerkleNode::from_byte_array([0; 32]),
            time: 1,
            bits: CompactTarget::from_consensus(0x207f_ffff),
            nonce: 0,
        };
        let txid = tx.compute_txid().to_byte_array();
        // Mismatch paths.
        assert!(matches!(
            block_to_apply_with_txids_prev(Fk::NULL, &header, std::slice::from_ref(&tx), &[]),
            Err(ConsensusError::BadBlock(_))
        ));
        assert!(matches!(
            block_to_apply_with_txids_prev(Fk::NULL, &header, &[], &[[0u8; 32]]),
            Err(ConsensusError::BadBlock(_))
        ));
        let (rec, apps) =
            block_to_apply_with_txids_prev(Fk::NULL, &header, &[tx], &[txid]).unwrap();
        assert!(rec.prev_fk.is_null());
        assert_eq!(apps.len(), 1);
        // Coinbase-like max vout on null prev with MAX index.
        let non_cb = Transaction {
            version: TxVersion::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint {
                    txid: bitcoin::Txid::from_byte_array([1; 32]),
                    vout: 3,
                },
                script_sig: ScriptBuf::new(),
                sequence: Sequence::MAX,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(1),
                script_pubkey: ScriptBuf::from_bytes(vec![0x51]),
            }],
        };
        let apply = tx_to_apply(&non_cb, non_cb.compute_txid().to_byte_array()).unwrap();
        assert_eq!(apply.inputs[0].prev_index, 3);
    }

    /// Write encode must bind each stamped spend edge to the wire prevout it
    /// claims to spend — a stale/mismatched edge is Corrupt, not encoded.
    #[test]
    fn input_encode_rejects_edge_wire_prevout_mismatch() {
        let tx = Transaction {
            version: TxVersion::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint {
                    txid: bitcoin::Txid::from_byte_array([1; 32]),
                    vout: 3,
                },
                script_sig: ScriptBuf::new(),
                sequence: Sequence::MAX,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(1),
                script_pubkey: ScriptBuf::from_bytes(vec![0x51]),
            }],
        };
        let edge = |prev_txid: [u8; 32], vout: u32| rbitcoin_query::SpendEdge {
            prev_txid,
            vout,
            spend_fk: Fk(9),
            create_fk: Fk(5),
            vin: 0,
        };
        let ok =
            rbitcoin_query::input_records_from_wire(&tx, Fk(9), &[edge([1u8; 32], 3)]).unwrap();
        assert_eq!(ok[0].prev_txid, [1u8; 32]);
        assert_eq!(ok[0].prev_index, 3);
        for bad in [edge([2u8; 32], 3), edge([1u8; 32], 4)] {
            let err = rbitcoin_query::input_records_from_wire(&tx, Fk(9), &[bad])
                .expect_err("mismatched edge must not encode");
            assert!(
                format!("{err}").contains("edge/wire prevout"),
                "unexpected error: {err}"
            );
        }
    }
}
#[cfg(test)]
mod pr3_kills_49 {
    use super::*;
    use bitcoin::{
        block::{Header, Version},
        BlockHash, CompactTarget, TxMerkleNode,
    };
    use rbitcoin_query::Query;
    use std::path::PathBuf;
    fn tq() -> (PathBuf, Query) {
        let p = std::env::temp_dir().join(format!(
            "rbitcoin-49-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&p).unwrap();
        (p.clone(), Query::open_or_create_tiny(&p).unwrap())
    }
    #[test]
    fn kills_49() {
        let (path, q) = tq();
        let hg = Header {
            version: Version::ONE,
            prev_blockhash: BlockHash::from_byte_array([0u8; 32]),
            merkle_root: TxMerkleNode::from_byte_array([0u8; 32]),
            time: 1,
            bits: CompactTarget::from_consensus(0x207fffff),
            nonce: 0,
        };
        assert!(block_to_apply_with_txids(&q, &hg, &[], &[]).is_ok());
        let hb = Header {
            version: Version::ONE,
            prev_blockhash: BlockHash::from_byte_array([1u8; 32]),
            merkle_root: TxMerkleNode::from_byte_array([0u8; 32]),
            time: 1,
            bits: CompactTarget::from_consensus(0x207fffff),
            nonce: 0,
        };
        assert!(block_to_apply_with_txids(&q, &hb, &[], &[]).is_err());
        let _ = std::fs::remove_dir_all(path);
    }
}
#[cfg(test)]
mod pr3_kills_86_87 {
    use super::*;
    use bitcoin::{
        absolute::LockTime, transaction::Version as TxVersion, Amount, OutPoint, ScriptBuf,
        Sequence, Transaction, TxIn, TxOut, Txid, Witness,
    };
    #[test]
    fn kills_86_87() {
        // coinbase
        let cb = TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::from_bytes(vec![0x00]),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        };
        let txc = Transaction {
            version: TxVersion::ONE,
            lock_time: LockTime::ZERO,
            input: vec![cb],
            output: vec![TxOut {
                value: Amount::from_sat(50),
                script_pubkey: ScriptBuf::from_bytes(vec![0x51]),
            }],
        };
        assert_eq!(
            tx_to_apply(&txc, [0u8; 32]).unwrap().inputs[0].prev_index,
            u32::MAX
        );

        // non-coinbase
        let nc = TxIn {
            previous_output: OutPoint {
                txid: Txid::from_byte_array([1u8; 32]),
                vout: 0,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        };
        let txn = Transaction {
            version: TxVersion::ONE,
            lock_time: LockTime::ZERO,
            input: vec![nc],
            output: vec![TxOut {
                value: Amount::from_sat(1),
                script_pubkey: ScriptBuf::from_bytes(vec![0x51]),
            }],
        };
        assert_eq!(
            tx_to_apply(&txn, [1u8; 32]).unwrap().inputs[0].prev_index,
            0
        );

        // EDGE that kills &&->|| and vout==->!=
        // txid 0, vout 0 : B=true, C=false
        // original B&&C=false => is_cb=false => 0
        // mutant B||C=true => is_cb=true => MAX
        // mutant B&&!C=true => is_cb=true => MAX
        let edge = TxIn {
            previous_output: OutPoint {
                txid: Txid::from_byte_array([0u8; 32]),
                vout: 0,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        };
        let txe = Transaction {
            version: TxVersion::ONE,
            lock_time: LockTime::ZERO,
            input: vec![edge],
            output: vec![TxOut {
                value: Amount::from_sat(1),
                script_pubkey: ScriptBuf::from_bytes(vec![0x51]),
            }],
        };
        let ae = tx_to_apply(&txe, [2u8; 32]).unwrap();
        println!(
            "edge is_null={} prev_index={}",
            OutPoint {
                txid: Txid::from_byte_array([0u8; 32]),
                vout: 0
            }
            .is_null(),
            ae.inputs[0].prev_index
        );
        assert_eq!(
            ae.inputs[0].prev_index, 0,
            "edge txid0 vout0 must be non-coinbase 0 - kills &&->|| and vout==->!="
        );

        // This also documents the 2 equivalent mutants:
        // ||->&& : A == (B&&C) so A||(B&&C) == A && (B&&C) == A
        // txid==->!= with vout MAX gives prev_index MAX in both cases -> not observable
    }
}
