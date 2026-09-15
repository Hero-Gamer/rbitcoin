#!/usr/bin/env bash
# Contract: LCOV gate ignores test files (not substring "test"), runs default
# workspace tests (Tier A IBD included), never-falls vs master, writes Shields
# JSON. Does not run llvm-cov.
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

base="$(mktemp)"
python3 "$ROOT/scripts/coverage-badge.py" \
  --lh 101175 --lf 110953 --gate 90 --sha b1510f783e3f --date 2026-09-15 --out "$base"

assert_gate_pass "equal ratio vs master baseline passes" \
  --lh 101175 --lf 110953 --baseline "$base"
assert_gate_fail "one fewer hit vs master fails (still ≥90%)" \
  --lh 101174 --lf 110953 --baseline "$base"
assert_gate_fail "90.50% vs 91.19% master fails" \
  --lh 905 --lf 1000 --baseline "$base"
assert_gate_pass "higher ratio vs master passes" \
  --lh 101586 --lf 111297 --baseline "$base"
assert_gate_pass "same ratio on a larger tree passes" \
  --lh 202350 --lf 221906 --baseline "$base"
assert_gate_pass "file:// baseline URL works" \
  --lh 101175 --lf 110953 --baseline "file://${base}"

assert_gate_pass "floor-only 90.00% passes" --lh 90 --lf 100 --floor-only
assert_gate_fail "floor-only 89.99% fails" --lh 8999 --lf 10000 --floor-only

if GITHUB_ACTIONS=true python3 "$GATE" --lh 90 --lf 100 \
  --baseline-url "http://127.0.0.1:1/coverage.json" >/dev/null 2>&1; then
  echo "not ok - CI fetch fail is an error (wanted fail)"
  FAIL=$((FAIL + 1))
else
  echo "ok - CI fetch fail is an error"
  PASS=$((PASS + 1))
fi
if env -u GITHUB_ACTIONS python3 "$GATE" --lh 90 --lf 100 \
  --baseline-url "http://127.0.0.1:1/coverage.json" >/dev/null; then
  echo "ok - local fetch fail falls back to 90% floor (pass)"
  PASS=$((PASS + 1))
else
  echo "not ok - local fetch fail falls back to 90% floor (pass)"
  FAIL=$((FAIL + 1))
fi
if env -u GITHUB_ACTIONS python3 "$GATE" --lh 89 --lf 100 \
  --baseline-url "http://127.0.0.1:1/coverage.json" >/dev/null 2>&1; then
  echo "not ok - local fetch fail falls back to 90% floor (fail under)"
  FAIL=$((FAIL + 1))
else
  echo "ok - local fetch fail falls back to 90% floor (fail under)"
  PASS=$((PASS + 1))
fi

st="$(mktemp)"
python3 "$GATE" --lh 101586 --lf 111297 --baseline "$base" --status-out "$st" >/dev/null
python3 - "$st" <<'PY'
import json, sys
from pathlib import Path
d = json.loads(Path(sys.argv[1]).read_text())
assert d["ok"] is True, d
assert d["lh"] == 101586 and d["lf"] == 111297, d
assert d["base_lh"] == 101175 and d["base_lf"] == 110953, d
assert d["mode"] == "ratchet", d
PY
assert_ok "status-out JSON names ratchet baseline" true
rm -f "$st" "$base"

# Merge-base ratchet: highest master snapshot at or before the fork point,
# not whatever origin/master has published while the PR was open.
gitrepo="$(mktemp -d)"
git -C "$gitrepo" init -q
git -C "$gitrepo" config user.email "gate@test"
git -C "$gitrepo" config user.name "gate"
echo a >"$gitrepo/f"
git -C "$gitrepo" add f
git -C "$gitrepo" commit -q -m A
SHA_A="$(git -C "$gitrepo" rev-parse HEAD)"
echo b >>"$gitrepo/f"
git -C "$gitrepo" commit -q -am B
SHA_B="$(git -C "$gitrepo" rev-parse HEAD)"
echo c >>"$gitrepo/f"
git -C "$gitrepo" commit -q -am C
SHA_C="$(git -C "$gitrepo" rev-parse HEAD)"
hist="$(mktemp)"
python3 - "$hist" "$SHA_A" "$SHA_B" "$SHA_C" <<'PY'
import json, sys
from pathlib import Path
path, a, b, c = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
rows = [
    {"sha": a, "lh": 900, "lf": 1000, "date": "2026-09-01"},
    {"sha": b, "lh": 910, "lf": 1000, "date": "2026-09-10"},
    {"sha": c, "lh": 930, "lf": 1000, "date": "2026-09-15"},
]
Path(path).write_text("".join(json.dumps(r) + "\n" for r in rows))
PY

python3 - "$GATE" "$hist" "$gitrepo" "$SHA_B" "$SHA_C" <<'PY' || exit 1
import importlib.util, json, sys
from pathlib import Path
gate_path, hist, gitrepo, sha_b, sha_c = sys.argv[1:6]
spec = importlib.util.spec_from_file_location("coverage_gate", gate_path)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)
entries = mod.parse_history(Path(hist).read_text())
picked = mod.pick_baseline(entries, sha_b, gitrepo)
assert picked is not None, "expected a baseline at B"
assert picked["lh"] == 910 and picked["sha"] == sha_b, picked
# C is after the fork; must not be the baseline for merge-base B.
assert picked["lh"] != 930
later = mod.pick_baseline(entries, sha_c, gitrepo)
assert later["lh"] == 930, later
print("ok - pick_baseline ignores post-fork master snapshots")
PY
assert_ok "pick_baseline ignores post-fork master snapshots" true

python3 - "$GATE" "$hist" "$gitrepo" "$SHA_B" "$SHA_A" <<'PY' || exit 1
import importlib.util, json, sys
from pathlib import Path
gate_path, hist, gitrepo, sha_b, sha_a = sys.argv[1:6]
spec = importlib.util.spec_from_file_location("coverage_gate", gate_path)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)
entries = mod.parse_history(Path(hist).read_text())
# Peak before B is A=90% vs B=91% → B wins. Rewrite A higher:
entries[0]["lh"] = 920
picked = mod.pick_baseline(entries, sha_b, gitrepo)
assert picked["lh"] == 920, picked
print("ok - pick_baseline takes the highest ancestor ratio, not the newest")
PY
assert_ok "pick_baseline takes the highest ancestor ratio, not the newest" true

tipbadge="$(mktemp)"
python3 "$ROOT/scripts/coverage-badge.py" \
  --lh 999 --lf 1000 --gate 90 --sha deadbeefdead --date 2026-09-15 --out "$tipbadge"
assert_gate_pass "CLI merge-base B does not require beating C" \
  --lh 910 --lf 1000 --history "$hist" --baseline "$tipbadge" \
  --merge-base "$SHA_B" --git-dir "$gitrepo"
assert_gate_fail "CLI merge-base B still fails a real drop vs B" \
  --lh 909 --lf 1000 --history "$hist" --baseline "$tipbadge" \
  --merge-base "$SHA_B" --git-dir "$gitrepo"
assert_gate_fail "CLI merge-base C still requires beating C" \
  --lh 920 --lf 1000 --history "$hist" --baseline "$tipbadge" \
  --merge-base "$SHA_C" --git-dir "$gitrepo"
rm -f "$tipbadge"

rm -f "$hist"
rm -rf "$gitrepo"

tmp="$(mktemp)"
python3 "$ROOT/scripts/coverage-badge.py" \
  --lh 905 --lf 1000 --gate 90 --base-lh 101175 --base-lf 110953 \
  --sha deadbeef --date 2026-09-15 --out "$tmp"
python3 - "$tmp" <<'PY'
import json, sys
from pathlib import Path
d = json.loads(Path(sys.argv[1]).read_text())
assert d["color"] == "red", d
assert d["base_lh"] == 101175, d
assert d["base_lf"] == 110953, d
PY
assert_ok "badge is red when below master even if ≥90%" true
rm -f "$tmp"

assert_ok "coverage.sh calls coverage-gate.py" \
  grep -q "coverage-gate.py" "$COV"

if [[ "$FAIL" -ne 0 ]]; then
  echo "coverage.test.sh: $PASS passed, $FAIL failed"
  exit 1
fi
echo "coverage.test.sh: $PASS passed"
