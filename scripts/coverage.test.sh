#!/usr/bin/env bash
# Contract: LCOV gate ignores test files (not substring "test"), runs default
# workspace tests (Tier A IBD included), 91% floor (no never-falls ratchet),
# writes Shields JSON. Does not run llvm-cov.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
COV="$ROOT/scripts/coverage.sh"
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

IGNORE="$(sed -n "s/^IGNORE='\\(.*\\)'/\\1/p" "$COV")"
if [ "${#IGNORE}" -eq 0 ]; then
  echo "not ok - coverage.sh defines IGNORE"
  exit 1
fi
echo "ok - coverage.sh defines IGNORE"
PASS=$((PASS + 1))

python3 - "$IGNORE" <<'PY' || exit 1
import re, sys
rx = re.compile(sys.argv[1])
excluded = [
    "/repo/crates/rbitcoin-store/src/scripthash_tests.rs",
    "/repo/crates/rbitcoin-net/src/ibd/confirm/tests.rs",
    "/repo/crates/rbitcoin-net/src/peer_tests.rs",
    "/repo/crates/rbitcoin-net/tests/ibd_smoke.rs",
    "/repo/crates/rbitcoin-test/src/lib.rs",
    "/repo/crates/rbitcoin-test/tests/integration_multinode.rs",
    "/repo/crates/rbitcoin-bench/src/suite.rs",
    "/repo/crates/rbitcoin-store/src/testutil.rs",
    "/repo/crates/rbitcoin-query/src/testutil.rs",
    "/repo/crates/rbitcoin-consensus/src/script/tests_verify.rs",
    "/repo/crates/rbitcoin-node/src/main.rs",
]
kept = [
    "/repo/crates/rbitcoin-store/src/scripthash.rs",
    "/repo/crates/rbitcoin-net/src/peer.rs",
    "/repo/crates/rbitcoin-net/src/ibd/mod.rs",
    "/repo/crates/rbitcoin-node/src/regtest_rpc.rs",
    "/repo/crates/rbitcoin-consensus/src/regtest_pad.rs",
    "/repo/crates/rbitcoin-node/src/run.rs",
    "/repo/crates/rbitcoin-rpc/src/methods/chain.rs",
]
fail = 0
for p in excluded:
    if rx.search(p) is None:
        print(f"not ok - should ignore {p}", file=sys.stderr)
        fail += 1
for p in kept:
    if rx.search(p) is not None:
        print(f"not ok - should count {p}", file=sys.stderr)
        fail += 1
sys.exit(fail)
PY
assert_ok "IGNORE drops test files and keeps production (incl. regtest_*)" true

assert_ok "two_node is not skipped" \
  bash -c '! grep -q "skip two_node_header_and_block_sync" "$1"' _ "$COV"
assert_ok "reconstruct is not skipped" \
  bash -c '! grep -q "skip serve_after_restart_via_reconstruct" "$1"' _ "$COV"
assert_ok "dead-peer is not skipped" \
  bash -c '! grep -q "skip ibd_skips_dead_peer" "$1"' _ "$COV"
assert_ok "llvm-cov test has no --skip" \
  bash -c '! grep -E "llvm-cov test" -A12 "$1" | grep -q -- "--skip"' _ "$COV"
assert_ok "llvm-cov excludes rbitcoin-bench" \
  bash -c 'grep -E "llvm-cov test" -A12 "$1" | grep -q -- "--exclude rbitcoin-bench"' _ "$COV"
assert_ok "llvm-cov test is GNU-timeout wrapped" \
  grep -q 'timeout --kill-after=30s' "$COV"
assert_ok "llvm-cov test does not pass --report-time (stable rustc)" \
  bash -c '! grep -E "report-time" "$1"' _ "$COV"
assert_ok "ci coverage job has a wall timeout" \
  bash -c 'awk "/^  coverage:/{p=1} p&&/timeout-minutes:/{print; exit}" "$1" | grep -q timeout-minutes' _ "$ROOT/.github/workflows/ci.yml"

tmp="$(mktemp)"
python3 "$ROOT/scripts/coverage-badge.py" \
  --lh 906 --lf 1000 --gate 90 --sha abcdef1234567890 --date 2026-09-13 --out "$tmp"
python3 - "$tmp" <<'PY'
import json, sys
from pathlib import Path
d = json.loads(Path(sys.argv[1]).read_text())
need = {
    "schemaVersion": 1,
    "label": "coverage",
    "message": "90.60%",
    "color": "brightgreen",
    "pct": 90.6,
    "lh": 906,
    "lf": 1000,
    "gate": 90,
    "scope": "production",
    "sha": "abcdef123456",
    "date": "2026-09-13",
}
for k, v in need.items():
    if d.get(k) != v:
        print(f"not ok - badge {k}: {d.get(k)!r} != {v!r}", file=sys.stderr)
        sys.exit(1)
PY
assert_ok "badge JSON is Shields endpoint + LH/LF extras" true
rm -f "$tmp"

tmp="$(mktemp)"
python3 "$ROOT/scripts/coverage-badge.py" \
  --lh 89 --lf 100 --gate 90 --sha deadbeef --date 2026-09-13 --out "$tmp"
python3 - "$tmp" <<'PY'
import json, sys
from pathlib import Path
d = json.loads(Path(sys.argv[1]).read_text())
assert d["color"] == "red", d["color"]
assert d["message"] == "89.00%", d["message"]
PY
assert_ok "badge is red below the gate" true
rm -f "$tmp"

out="$(BADGE_DRY_RUN=1 "$ROOT/scripts/publish-coverage-badge.sh")"
assert_ok "publish dry-run names badges/coverage.json" \
  grep -q "badges/coverage.json" <<<"$out"

GATE="$ROOT/scripts/coverage-gate.py"
assert_ok "coverage-gate.py exists" test -f "$GATE"

assert_gate_pass() {
  local name="$1"
  shift
  if python3 "$GATE" "$@" >/dev/null; then
    echo "ok - $name"
    PASS=$((PASS + 1))
  else
    echo "not ok - $name"
    FAIL=$((FAIL + 1))
  fi
}

assert_gate_fail() {
  local name="$1"
  shift
  if python3 "$GATE" "$@" >/dev/null 2>&1; then
    echo "not ok - $name (wanted fail)"
    FAIL=$((FAIL + 1))
  else
    echo "ok - $name"
    PASS=$((PASS + 1))
  fi
}

assert_gate_pass "91.00% floor passes" --lh 91 --lf 100
assert_gate_fail "90.99% floor fails" --lh 9099 --lf 10000
assert_gate_pass "production-scale 91.27% passes the 91% floor" \
  --lh 104220 --lf 114185
assert_gate_fail "90.00% fails the 91% floor" --lh 90 --lf 100

st="$(mktemp)"
python3 "$GATE" --lh 104220 --lf 114185 --status-out "$st" >/dev/null
python3 - "$st" <<'PY'
import json, sys
from pathlib import Path
d = json.loads(Path(sys.argv[1]).read_text())
assert d["ok"] is True, d
assert d["mode"] == "floor", d
assert d["floor"] == 91, d
PY
assert_ok "status-out JSON names 91% floor" true
rm -f "$st"

assert_ok "coverage.sh calls coverage-gate.py" \
  grep -q "coverage-gate.py" "$COV"
assert_ok "coverage.sh does not fetch merge-base for the gate" \
  bash -c '! grep -q "Coverage merge-base" "$1"' _ "$COV"

if [[ "$FAIL" -ne 0 ]]; then
  echo "coverage.test.sh: $PASS passed, $FAIL failed"
  exit 1
fi
echo "coverage.test.sh: $PASS passed"
