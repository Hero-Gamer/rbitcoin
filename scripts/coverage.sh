#!/usr/bin/env bash
# Enforce production LCOV LH/LF never falls vs last green master (90% floor).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

export CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-always}"

# Instrumented objects must not share a target dir with plain cargo test /
# clippy (different RUSTFLAGS fingerprints thrash rebuilds). Dev shell uses
# target/dev; musl release is nix/crane only.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR_COV:-$ROOT/target/cov}"
echo "coverage: CARGO_TARGET_DIR=$CARGO_TARGET_DIR"

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo not found; enter nix-shell first" >&2
  exit 1
fi

# Core JSON corpora are staged from the v31.1 submodule (not in-tree copies).
./scripts/core-functional/init-submodule.sh

# Ensure bins exist for binary smoke scenarios (instrumented tree).
cargo build -p rbitcoin-node -p rbitcoin-cli

# Prefer system llvm-tools when rustup component is unavailable (Nix).
if [[ -z "${LLVM_COV:-}" ]] && command -v llvm-cov >/dev/null 2>&1; then
  export LLVM_COV="$(command -v llvm-cov)"
fi
if [[ -z "${LLVM_PROFDATA:-}" ]] && command -v llvm-profdata >/dev/null 2>&1; then
  export LLVM_PROFDATA="$(command -v llvm-profdata)"
fi

# main.rs trampolines are one-liners; logic is covered via cli_main in libs.
# Test files / rbitcoin-test / rbitcoin-bench / testutil are not production. Do not match the
# substring "test" (regtest_rpc.rs / regtest_pad.rs stay in the denominator).
IGNORE='(/\.cargo/|/rustc-|/nix/store/|library/std/|/src/main\.rs$|/tests/|_tests\.rs$|/tests\.rs$|/testutil\.rs$|/tests_verify\.rs$|/crates/rbitcoin-test/|/crates/rbitcoin-bench/)'

if command -v cargo-llvm-cov >/dev/null 2>&1 || cargo llvm-cov --version >/dev/null 2>&1; then
  ./scripts/coverage.test.sh
  # Default: do NOT clean instrumented artifacts. Incremental llvm-cov rebuilds
  # are much faster for iterative work and still re-run all tests with coverage.
  # Force a full clean when debugging stale counters: COVERAGE_CLEAN=1 ./scripts/coverage.sh
  if [[ "${COVERAGE_CLEAN:-0}" == "1" ]]; then
    echo "COVERAGE_CLEAN=1: wiping llvm-cov workspace artifacts"
    cargo llvm-cov clean --workspace
  fi

  # Branch coverage requires nightly on many toolchains; line gate is always on.
  EXTRA=()
  if cargo llvm-cov test --help 2>&1 | grep -q -- '--fail-under-branches'; then
    if rustc -vV 2>/dev/null | grep -q nightly; then
      EXTRA+=(--branch --fail-under-branches 90)
    fi
  fi
  mkdir -p "$ROOT/coverage"
  cargo llvm-cov test --workspace --exclude rbitcoin-bench \
    --ignore-filename-regex "$IGNORE" \
    "${EXTRA[@]}" \
    --html --output-dir "$ROOT/coverage"

  REPORT="$(cargo llvm-cov report --ignore-filename-regex "$IGNORE" 2>/dev/null || true)"
  echo "$REPORT"

  cargo llvm-cov report \
    --ignore-filename-regex "$IGNORE" \
    --lcov --output-path "$ROOT/coverage/lcov.info" || true

  # Line gate: LCOV LH/LF never-falls vs last green master (90% floor).
  LCOV_STATS="$(python3 - <<'PY'
from pathlib import Path
p = Path("coverage/lcov.info")
lf = lh = 0
if p.exists():
    for line in p.read_text(errors="replace").splitlines():
        if line.startswith("LF:"):
            lf += int(line[3:])
        elif line.startswith("LH:"):
            lh += int(line[3:])
print(f"{lh} {lf}")
PY
)"
  read -r LCOV_HIT LCOV_TOT <<<"$LCOV_STATS"
  LCOV_HIT="${LCOV_HIT:-0}"
  LCOV_TOT="${LCOV_TOT:-0}"
  if [[ "$LCOV_TOT" -le 0 ]]; then
    echo "FAIL: no LCOV totals (missing coverage/lcov.info or empty LF)" >&2
    exit 1
  fi
  # Display and gate both use 2-decimal percent (llvm-cov LH jitters).
  LCOV_PCT="$(python3 -c "print(f'{100.0*$LCOV_HIT/$LCOV_TOT:.2f}')")"
  MISS=$((LCOV_TOT > LCOV_HIT ? LCOV_TOT - LCOV_HIT : 0))
  echo "LCOV lines: ${LCOV_HIT}/${LCOV_TOT} (${LCOV_PCT}%) miss=${MISS} (production files)"
  echo "Line coverage gate: never below highest master coverage at or before merge-base; ${LCOV_PCT}% now"
  echo "Gate math: pass iff displayed 2-decimal % does not fall (llvm-cov LH jitters; 90% floor if no snapshot)"

  # Optional HTML diagnostic (not the pass condition).
  HTML_PRESENT=0
  if find "$ROOT/coverage" -name '*.html' -print -quit 2>/dev/null | grep -q .; then
    HTML_PRESENT=1
  fi
  if [[ "$HTML_PRESENT" -eq 1 ]]; then
    UNCOV_TOTAL="$(
      python3 - <<'PY'
from pathlib import Path
import re
n = 0
for p in Path("coverage").rglob("*.html"):
    t = p.read_text(errors="replace")
    n += len(re.findall(r"class=['\"]uncovered-line['\"]", t))
print(n)
PY
    )"
    echo "HTML uncovered-line markers (diagnostic): ${UNCOV_TOTAL}"
  fi

  # Unrounded LH/LF vs highest master snapshot at or before merge-base.
  GATE_JSON="$ROOT/coverage/gate.json"
  GATE_ARGS=(--lh "$LCOV_HIT" --lf "$LCOV_TOT" --status-out "$GATE_JSON")
  MERGE_BASE="${COVERAGE_MERGE_BASE:-}"
  if [[ -z "$MERGE_BASE" ]] && git -C "$ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    git -C "$ROOT" fetch --no-tags origin master:refs/remotes/origin/master >/dev/null 2>&1 || true
    HEAD_FOR_BASE="${COVERAGE_HEAD_SHA:-${GITHUB_HEAD_SHA:-HEAD}}"
    MERGE_BASE="$(git -C "$ROOT" merge-base origin/master "$HEAD_FOR_BASE" 2>/dev/null || true)"
  fi
  if [[ -n "$MERGE_BASE" ]]; then
    echo "Coverage merge-base: ${MERGE_BASE}"
    GATE_ARGS+=(--merge-base "$MERGE_BASE" --git-dir "$ROOT")
  fi
  if ! python3 "$ROOT/scripts/coverage-gate.py" "${GATE_ARGS[@]}"; then
    cargo llvm-cov report --ignore-filename-regex "$IGNORE" --show-missing-lines || true
    exit 1
  fi
  SHA="${GITHUB_SHA:-$(git rev-parse HEAD)}"
  BADGE_EXTRA=()
  BASE_LH="$(python3 -c "import json; print(json.load(open('$GATE_JSON')).get('base_lh',''))")"
  BASE_LF="$(python3 -c "import json; print(json.load(open('$GATE_JSON')).get('base_lf',''))")"
  if [[ -n "$BASE_LH" && -n "$BASE_LF" ]]; then
    BADGE_EXTRA+=(--base-lh "$BASE_LH" --base-lf "$BASE_LF")
  fi
  python3 "$ROOT/scripts/coverage-badge.py" \
    --lh "$LCOV_HIT" --lf "$LCOV_TOT" --gate 90 \
    --sha "$SHA" --scope production --out "$ROOT/coverage/badge.json" \
    "${BADGE_EXTRA[@]}"
  echo "Wrote coverage/badge.json (${LCOV_PCT}% production)"
  echo "Note: full branch coverage requires nightly --branch; region-partial lines may still appear in text report."
  echo "Tip: set COVERAGE_CLEAN=1 only when you need a cold instrumented rebuild."
  echo "Tooling: use llvmPackages matching rustc (rustc 1.95 → LLVM 21; see shell.nix)."
  ./scripts/coverage-crap.sh "$ROOT/coverage/lcov.info" "$ROOT/coverage/crap.json"
  exit 0
fi

echo "cargo-llvm-cov not installed; running tests without coverage gate." >&2
echo "Install: cargo install cargo-llvm-cov --locked" >&2
echo "Or: nix-shell -p cargo-llvm-cov" >&2
cargo test --workspace
echo "WARNING: coverage gate skipped (tooling missing)" >&2
exit 0
