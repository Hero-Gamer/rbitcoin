//! Regression tests from fuzz-discovered edge cases
//! Each input = a case that survives existing coverage → must be caught here

use bitcoin::script::ScriptBuf;
use std::fs;
use std::path::Path;

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
}

/// Parse hex string to bytes — matches pattern in script_edge_fixtures.rs
fn hex_bytes(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .collect::<String>()
        .as_bytes()
        .chunks(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}

/// Verifies the concrete fuzz-discovered fixture exists with correct content
#[test]
fn fixture_invalid_pushdata_present_and_correct() {
    let fixture =
        repo_root().join("fuzz/promoted/regression/script-parsing/invalid_op_push_negative");

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
        "Fixture hex mismatch — expected the malformed PUSHDATA4 pattern"
    );
}

/// Documents the known gap: standard Script parsing stops at OP_RETURN
/// and does NOT validate the impossible PUSHDATA4 length that follows.
/// Validation MUST happen at the rbitcoin consensus/check level.
#[test]
fn standard_parser_does_not_reject_impossible_pushdata4() {
    // Pattern: 6a ff ff ff ff ff
    // 6a = OP_RETURN → standard parser stops here, ignores the rest
    // ff ff ff ff ff = would-be PUSHDATA4 length field → impossible value
    // This is the EXACT gap fuzzing surfaced:
    // bitcoin::script::ScriptBuf accepts it silently → consensus MUST catch it
    let bytes = hex_bytes("6a ff ff ff ff ff");
    let script = ScriptBuf::from_bytes(bytes);
    let mut iter = script.instructions();

    // Confirmed: standard parser does NOT reject — returns Ok(OP_RETURN)
    // This test locks in the KNOWN behavior so we know when it changes upstream
    assert!(
        matches!(iter.next(), Some(Ok(_))),
        "Standard script parser accepts this — gap confirmed"
    );

    // The actual rejection belongs in rbitcoin consensus validation,
    // NOT in the bitcoin library's instruction parser.
    // TODO: add the consensus-level check here once validation path confirmed.
}
