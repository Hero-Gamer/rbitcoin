#!/usr/bin/env bash
# Contract: CRAP after LCOV is workspace summary + json file sort +
# --fail-above --threshold 30. Allowlist lives in .cargo-crap.toml.
# Never --fail-regression. Does not run llvm-cov.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RUN="$ROOT/scripts/coverage-crap.sh"
PASS=0
FAIL=0

assert_ok() {
  local name="$1"
  shift
  if "$@"; then
    echo "ok - $name"
    PASS=$((PASS + 1))
  else
    echo "not ok - $name"
    FAIL=$((FAIL + 1))
  fi
}

out="$(CRAP_DRY_RUN=1 "$RUN")"
assert_ok "dry-run uses cargo crap --workspace" \
  grep -q "cargo crap --workspace" <<<"$out"
assert_ok "dry-run reads coverage/lcov.info" \
  grep -q -- "--lcov " <<<"$out" && grep -q "coverage/lcov.info" <<<"$out"
assert_ok "dry-run prints --summary" \
  grep -q -- "--summary" <<<"$out"
assert_ok "dry-run writes coverage/crap.json" \
  grep -q "coverage/crap.json" <<<"$out"
assert_ok "dry-run json is --format json --sort file" \
  grep -q -- "--format json" <<<"$out" && grep -q -- "--sort file" <<<"$out"
assert_ok "dry-run gates --fail-above --threshold 30" \
  grep -q -- "--fail-above" <<<"$out" && grep -q -- "--threshold 30" <<<"$out"
assert_ok "dry-run has no --fail-regression" \
  bash -c '! grep -q -- "--fail-regression" <<<"$1"' _ "$out"
json_line="$(grep -- "--format json" <<<"$out")"
assert_ok "json artifact line is not the fail-above gate" \
  bash -c '! grep -q -- "--fail-above" <<<"$1"' _ "$json_line"
assert_ok "dry-run does not invoke llvm-cov" \
  bash -c '! grep -q llvm-cov <<<"$1"' _ "$out"
assert_ok ".cargo-crap.toml exists" test -f "$ROOT/.cargo-crap.toml"
assert_ok ".cargo-crap.toml allowlists production offenders" \
  grep -q "run_p2p" "$ROOT/.cargo-crap.toml"
assert_ok ".cargo-crap.toml excludes rbitcoin-bench" \
  grep -q "rbitcoin-bench" "$ROOT/.cargo-crap.toml"

if [[ "$FAIL" -ne 0 ]]; then
  echo "coverage-crap.test.sh: $PASS passed, $FAIL failed"
  exit 1
fi
echo "coverage-crap.test.sh: $PASS passed"
