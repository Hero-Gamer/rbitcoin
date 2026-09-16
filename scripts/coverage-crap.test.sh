#!/usr/bin/env bash
# Contract: CRAP after LCOV is workspace summary + json file sort.
# With crap_baseline.json: --fail-regression. Never --fail-above.
# Does not run llvm-cov.
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
assert_ok "dry-run has no --fail-above" \
  bash -c '! grep -q -- "--fail-above" <<<"$1"' _ "$out"
assert_ok "dry-run without baseline has no --fail-regression" \
  bash -c '! grep -q -- "--fail-regression" <<<"$1"' _ "$out"

base="$(mktemp)"
echo '{"$schema":"https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/report-v1.json","version":"0.4.3","entries":[]}' >"$base"
gated="$(CRAP_DRY_RUN=1 CRAP_BASELINE="$base" "$RUN")"
rm -f "$base"
json_line="$(grep -- "--format json" <<<"$gated")"
gate_line="$(grep -- "--fail-regression" <<<"$gated")"
assert_ok "dry-run with baseline uses --fail-regression" \
  grep -q -- "--fail-regression" <<<"$gated"
assert_ok "dry-run with baseline passes --baseline" \
  grep -q -- "--baseline " <<<"$gated"
assert_ok "json artifact line is not the regression gate" \
  bash -c '! grep -q -- "--fail-regression" <<<"$1"' _ "$json_line"
assert_ok "gate line is not --format json" \
  bash -c '! grep -q -- "--format json" <<<"$1"' _ "$gate_line"
assert_ok "gated dry-run still has no --fail-above" \
  bash -c '! grep -q -- "--fail-above" <<<"$1"' _ "$gated"
assert_ok "dry-run does not invoke llvm-cov" \
  bash -c '! grep -q llvm-cov <<<"$1"' _ "$out"

if [[ "$FAIL" -ne 0 ]]; then
  echo "coverage-crap.test.sh: $PASS passed, $FAIL failed"
  exit 1
fi
echo "coverage-crap.test.sh: $PASS passed"
