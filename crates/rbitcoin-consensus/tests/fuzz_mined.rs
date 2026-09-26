//! Regression tests from fuzz-discovered edge cases
//! Each input = a case that survives existing coverage → MUST be rejected by rbitcoin consensus

use bitcoin::{
    absolute::LockTime, transaction::Version, Amount, OutPoint, ScriptBuf, Sequence, Transaction,
    TxIn, TxOut, Witness,
};
use rbitcoin_consensus::verify_tx_scripts_detached_forks;
use std::fs;
use std::path::Path;

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
}

/// Parse hex string to bytes
fn hex_bytes(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .collect::<String>()
        .as_bytes()
        .chunks(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}

/// Fixture exists with correct hex pattern
#[test]
fn fixture_op_return_divergence_present_and_correct() {
    let fixture = repo_root().join("fuzz/promoted/regression/script-parsing/op_return_divergence");

    assert!(
        fixture.exists(),
        "Fixture missing: expected at {:?}",
        fixture
    );

    let content = fs::read_to_string(&fixture).expect("Failed to read fixture file");

    let hex_line = content
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .expect("Fixture should have non-comment hex data line");

    assert_eq!(
        hex_line, "6a ff ff ff ff ff",
        "Fixture hex mismatch — expected the pattern that triggers divergent behavior"
    );
}

/// rbitcoin consensus REJECTS scripts containing OP_RETURN
/// The bitcoin crate parser accepts it as valid OP_RETURN — this divergence
/// is exactly what fuzzing surfaced. rbitcoin interpreter returns Err immediately.
#[test]
fn op_return_script_is_rejected_by_rbitcoin_consensus() {
    // Pattern: 6a ff ff ff ff ff
    // 6a = OP_RETURN → rbitcoin interpreter rejects immediately
    let script_sig_bytes = hex_bytes("6a ff ff ff ff ff");

    // Build minimal transaction
    let tx = Transaction {
        version: Version(1),
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::default(),
            script_sig: ScriptBuf::from(script_sig_bytes),
            sequence: Sequence::MAX,
            witness: Witness::default(),
        }],
        output: vec![TxOut {
            value: Amount::ZERO,
            script_pubkey: ScriptBuf::new(),
        }],
    };

    // Pass by value — matches function signature exactly
    let empty_prevouts = Vec::new();
    let result = verify_tx_scripts_detached_forks(
        empty_prevouts, // 1: prevouts
        tx,             // 2: transaction
        true,           // 3: allow_checkpoint
        true,           // 4: verify_sigops
        true,           // 5: allow_witness
        true,           // 6: is_standard
        true,           // 7: taproot_active
    );

    assert!(
        result.is_err(),
        "rbitcoin consensus MUST reject script containing OP_RETURN — diverges from bitcoin crate parser"
    );
}
