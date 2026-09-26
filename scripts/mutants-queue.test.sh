#!/usr/bin/env bash
# Cursor math for the nightly mutants queue. Does not run cargo-mutants.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PY="$ROOT/scripts/mutants_queue.py"
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

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

cat >"$tmp/list.txt" <<'EOF'
crates/a/src/lib.rs:2:1: replace a in old
crates/b/src/lib.rs:4:1: replace b in new
crates/a/src/lib.rs:9:1: replace c in old
not a mutant
EOF

cat >"$tmp/diff.txt" <<'EOF'
diff --git a/crates/b/src/lib.rs b/crates/b/src/lib.rs
--- a/crates/b/src/lib.rs
+++ b/crates/b/src/lib.rs
@@ -3,0 +4 @@
+fn new_line() {}
EOF

order="$(python3 "$PY" order --list "$tmp/list.txt" --diff "$tmp/diff.txt" --old-index 1 --new-skip 0)"
first="$(printf '%s\n' "$order" | head -n 1)"
second="$(printf '%s\n' "$order" | sed -n '2p')"
assert_ok "new mutant is first" test "$first" = "crates/b/src/lib.rs:4:1: replace b in new"
assert_ok "old rotation starts at index 1" test "$second" = "crates/a/src/lib.rs:9:1: replace c in old"

skipped="$(python3 "$PY" order --list "$tmp/list.txt" --diff "$tmp/diff.txt" --old-index 0 --new-skip 1 | head -n 1)"
assert_ok "new-skip drops the new prefix" test "$skipped" = "crates/a/src/lib.rs:2:1: replace a in old"

printf '%s\n' '{"new_base":"abc","new_skip":0,"old_index":1}' >"$tmp/cursor.json"
python3 "$PY" advance --cursor "$tmp/cursor.json" --n-new 2 --n-old 2 --completed 1 --head abc
assert_ok "partial new keeps the base" grep -q '"new_base": "abc"' "$tmp/cursor.json"
assert_ok "partial new records skip" grep -q '"new_skip": 1' "$tmp/cursor.json"

python3 "$PY" advance --cursor "$tmp/cursor.json" --n-new 2 --n-old 2 --completed 2 --head def
assert_ok "finished new moves the base" grep -q '"new_base": "def"' "$tmp/cursor.json"
assert_ok "finished new clears skip" grep -q '"new_skip": 0' "$tmp/cursor.json"
# old_index was 1; the extra completed mutant walks one old slot -> 0 (mod 2)
assert_ok "old index wraps" grep -q '"old_index": 0' "$tmp/cursor.json"

angled='crates/rbitcoin-esplora/src/handlers.rs:1504:5: replace sat_vb -> Option<f64> with None'
re_out="$(python3 "$PY" re "$angled")"
assert_ok "regex is anchored" bash -c '[[ "$1" == ^* && "$1" == *\$ ]]' _ "$re_out"
assert_ok "angle brackets stay literal" bash -c 'printf "%s\n" "$1" | grep -F -q "Option<f64>"' _ "$re_out"
assert_ok "hyphen and dot are escaped" bash -c 'printf "%s\n" "$1" | grep -F -q "rbitcoin\\-esplora" && printf "%s\n" "$1" | grep -F -q "handlers\\.rs"' _ "$re_out"

pipes='crates/a.rs:1:1: replace && with || in f'
pipe_re="$(python3 "$PY" re "$pipes")"
assert_ok "pipes are escaped" bash -c '[[ "$1" == *\\\|\\\|* ]]' _ "$pipe_re"

cat >"$tmp/batch.log" <<'EOF'
Found 22 mutants to test
22 mutants tested in 11m: 10 caught, 12 unviable
EOF
finished="$(python3 "$PY" finished --log "$tmp/batch.log")"
assert_ok "summary counts caught and unviable" test "$finished" = 22

cat >"$tmp/partial.log" <<'EOF'
MISSED   crates/a.rs:1:1: replace == with != in f
caught   crates/a.rs:2:1: replace x with y in f
EOF
partial="$(python3 "$PY" finished --log "$tmp/partial.log")"
assert_ok "killed batch counts outcome lines" test "$partial" = 2

NIGHTLY="$ROOT/scripts/mutants-nightly.sh"
TOML="$ROOT/.cargo/mutants.toml"
assert_ok "nightly list excludes rbitcoin-bench" \
  bash -c 'grep -q -- "--exclude '"'"'crates/rbitcoin-bench/\*\*/\*.rs'"'"' --list" "$1"' _ "$NIGHTLY"
assert_ok "nightly run excludes rbitcoin-bench" \
  bash -c 'grep -q -- "--exclude '"'"'crates/rbitcoin-bench/\*\*/\*.rs'"'"'" "$1"' _ "$NIGHTLY"
assert_ok "mutants.toml excludes rbitcoin-bench" \
  grep -q 'crates/rbitcoin-bench/\*\*/\*.rs' "$TOML"
WF="$ROOT/.github/workflows/mutants.yml"
assert_ok "nightly cron is 03:47 UTC" \
  grep -q 'cron: "47 3 \* \* \*"' "$WF"
assert_ok "nightly uses the queue regex helper" \
  grep -q 'mutants_queue.py" re' "$NIGHTLY"
assert_ok "nightly counts the mutants summary" \
  grep -q 'mutants_queue.py" finished' "$NIGHTLY"
assert_ok "same-file cursor copy is skipped" \
  grep -q -- '-ef' "$NIGHTLY"
assert_ok "job timeout is 330 minutes" \
  grep -q 'timeout-minutes: 330' "$WF"
assert_ok "script budget is 5 hours" \
  grep -q 'MUTANTS_BUDGET_SEC:-18000' "$NIGHTLY"

echo "$PASS passed, $FAIL failed"
test "$FAIL" -eq 0
