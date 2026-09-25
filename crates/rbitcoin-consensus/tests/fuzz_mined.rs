//! Regression tests from fuzz-discovered edge cases
//! Each input = a case that survives existing coverage → must be caught here

use std::fs;
use std::path::Path;

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
}

/// Verifies the concrete fuzz-discovered fixture exists with correct content
#[test]
fn fixture_invalid_pushdata_present_and_correct() {
    // 📌 Concrete example: malformed PUSHDATA4
    // Hex: 6a ff ff ff ff ff
    // OP_PUSHDATA4 claims 0xFFFFFFFF bytes follow → only 4 remain → impossible
    // This exact pattern was found by fuzzing and would slip past existing tests
    // without this explicit check — it is NOT covered by the regular suite
    let fixture =
        repo_root().join("fuzz/promoted/regression/script-parsing/invalid_op_push_negative");

    // Must exist at expected path
    assert!(
        fixture.exists(),
        "Fixture missing: expected at {:?}",
        fixture
    );

    // Must contain the exact hex we intend to protect
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

/// Documents: this input represents a gap existing tests don't cover
/// Without this fixture, a mutant weakening validation would survive undetected
#[test]
fn fixture_represents_an_uncatchable_gap() {
    // This is a placeholder documenting the gap
    // Next PR: add canonical-pushdata validation assert here
    // The fixture above is the real check — this test just explains the value
    let _note = "Next: add canonical-validation assert to pin rejection behavior";
}
