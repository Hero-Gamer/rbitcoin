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

cat >"$tmp/queue.txt" <<'EOF'
crates/a/src/lib.rs:1:1: replace a in f
crates/a/src/lib.rs:2:1: replace b in f
crates/b/src/lib.rs:3:1: replace c in f
EOF
batch="$(python3 "$PY" file-batch --queue "$tmp/queue.txt" --offset 0 --cap 10)"
assert_ok "file batch stays on the first path" test "$batch" = "$(printf '%s\n' \
  'crates/a/src/lib.rs:1:1: replace a in f' \
  'crates/a/src/lib.rs:2:1: replace b in f')"
capped="$(python3 "$PY" file-batch --queue "$tmp/queue.txt" --offset 0 --cap 1)"
assert_ok "file batch honors the cap" test "$capped" = "crates/a/src/lib.rs:1:1: replace a in f"
next_file="$(python3 "$PY" file-batch --queue "$tmp/queue.txt" --offset 2 --cap 10)"
assert_ok "file batch offset reaches the next path" test "$next_file" = "crates/b/src/lib.rs:3:1: replace c in f"

cat >"$tmp/slice.txt" <<'EOF'
crates/a/src/lib.rs:2:1: replace b in f
crates/a/src/lib.rs:4:1: replace d in f
EOF
cat >"$tmp/killed.log" <<'EOF'
unviable crates/a/src/lib.rs:1:1: delete field x from struct S expression in f in 1s build
caught   crates/a/src/lib.rs:2:1: replace b in f in 10s build + 4s test
caught   crates/a/src/lib.rs:9:1: replace z in f in 10s build + 4s test
EOF
prog="$(python3 "$PY" progress --log "$tmp/killed.log" --slice "$tmp/slice.txt")"
assert_ok "progress ignores unfiltered deletes and stops at a hole" test "$prog" = 1

# Half the job budget is the new-mutant window. A batch already started
# may finish after that mark; the next batch is backlog. Names below are
# one file each so a cap of 1 is one cargo invocation.
cat >"$tmp/new-q.txt" <<'EOF'
crates/new/a.rs:1:1: replace a with b in n1
crates/new/b.rs:2:1: replace a with b in n2
crates/new/c.rs:3:1: replace a with b in n3
crates/new/d.rs:4:1: replace a with b in n4
EOF
cat >"$tmp/old-q.txt" <<'EOF'
crates/old/a.rs:1:1: replace a with b in o1
crates/old/b.rs:2:1: replace a with b in o2
crates/old/c.rs:3:1: replace a with b in o3
crates/old/d.rs:4:1: replace a with b in o4
crates/old/e.rs:5:1: replace a with b in o5
crates/old/f.rs:6:1: replace a with b in o6
EOF

sched="$(python3 "$PY" schedule --new "$tmp/new-q.txt" --old "$tmp/old-q.txt" \
  --budget 1000 --batch-sec 200 --cap 1)"
assert_ok "both sides: after half the budget the names are backlog" \
  python3 -c '
import sys
rows = [line.split("\t", 2) for line in sys.argv[1].splitlines()]
half = 500
after = [r for r in rows if int(r[1]) >= half]
before = [r for r in rows if int(r[1]) < half]
assert after and all(r[0] == "old" for r in after), after
assert before and all(r[0] == "new" for r in before), before
assert any(r[0] == "new" and int(r[1]) + 200 > half for r in before), before
' "$sched"

cross="$(python3 "$PY" schedule --new "$tmp/new-q.txt" --old "$tmp/old-q.txt" \
  --budget 1000 --batch-sec 400 --cap 1)"
assert_ok "a batch that started before half stays new" \
  python3 -c '
import sys
rows = [line.split("\t", 2) for line in sys.argv[1].splitlines()]
by = {int(r[1]): r[0] for r in rows}
assert by[0] == "new" and by[400] == "new" and by[800] == "old", rows
' "$cross"

cat >"$tmp/one-new.txt" <<'EOF'
crates/new/a.rs:1:1: replace a with b in n1
EOF
early="$(python3 "$PY" schedule --new "$tmp/one-new.txt" --old "$tmp/old-q.txt" \
  --budget 1000 --batch-sec 200 --cap 1)"
assert_ok "unused new time goes to the backlog immediately" \
  python3 -c '
import sys
rows = [line.split("\t", 2) for line in sys.argv[1].splitlines()]
assert rows[0][0] == "new" and int(rows[0][1]) == 0, rows
assert rows[1][0] == "old" and int(rows[1][1]) == 200, rows
' "$early"

: >"$tmp/empty-q.txt"
empty_new="$(python3 "$PY" schedule --new "$tmp/empty-q.txt" --old "$tmp/old-q.txt" \
  --budget 1000 --batch-sec 200 --cap 1)"
assert_ok "empty new schedules backlog immediately" \
  python3 -c '
import sys
rows = [line.split("\t", 2) for line in sys.argv[1].splitlines()]
assert rows and rows[0][0] == "old" and int(rows[0][1]) == 0, rows
assert all(r[0] == "old" for r in rows), rows
' "$empty_new"

empty_old="$(python3 "$PY" schedule --new "$tmp/new-q.txt" --old "$tmp/empty-q.txt" \
  --budget 1000 --batch-sec 200 --cap 1)"
assert_ok "empty backlog keeps scheduling new past half" \
  python3 -c '
import sys
rows = [line.split("\t", 2) for line in sys.argv[1].splitlines()]
assert rows and all(r[0] == "new" for r in rows), rows
assert any(int(r[1]) >= 500 for r in rows), rows
' "$empty_old"

capped="$(python3 "$PY" schedule --new "$tmp/new-q.txt" --old "$tmp/old-q.txt" \
  --budget 1000 --batch-sec 200 --cap 1 --new-cap-used 1)"
assert_ok "a used new cap does not schedule more new while backlog remains" \
  python3 -c '
import sys
rows = [line.split("\t", 2) for line in sys.argv[1].splitlines()]
assert rows and all(r[0] == "old" for r in rows), rows
assert not any(r[0] == "new" for r in rows), rows
' "$capped"

kept="$(python3 "$PY" resume-old --list "$tmp/old-q.txt" --index 0 \
  --name "crates/old/c.rs:3:1: replace a with b in o3")"
assert_ok "a persisted backlog name that still exists is the resume point" \
  test "$(printf '%s\n' "$kept" | head -n 1)" = "crates/old/c.rs:3:1: replace a with b in o3"

missing="$(python3 "$PY" resume-old --list "$tmp/old-q.txt" --index 5 \
  --name "crates/gone.rs:1:1: replace a with b in gone")"
assert_ok "a missing backlog name resumes at the saved index modulo length" \
  test "$(printf '%s\n' "$missing" | head -n 1)" = "crates/old/f.rs:6:1: replace a with b in o6"
assert_ok "a missing backlog name does not resume at zero" \
  test "$(printf '%s\n' "$missing" | head -n 1)" != "crates/old/a.rs:1:1: replace a with b in o1"

printf '%s\n' '{"new_base":"abc","new_skip":0,"old_index":0,"old_name":"","new_cap_used":true,"run_id":"42"}' >"$tmp/night.json"
python3 "$PY" night --cursor "$tmp/night.json" --run-id 42
assert_ok "the same night keeps a consumed new cap" \
  grep -q '"new_cap_used": true' "$tmp/night.json"
python3 "$PY" night --cursor "$tmp/night.json" --run-id 99
assert_ok "a new night clears the new cap" \
  grep -q '"new_cap_used": false' "$tmp/night.json"
assert_ok "a new night records its run id" \
  grep -q '"run_id": "99"' "$tmp/night.json"

printf '%s\n' '{"new_base":"","new_skip":0,"old_index":1,"old_name":"crates/old/c.rs:3:1: replace a with b in o3","new_cap_used":false,"run_id":""}' >"$tmp/split-cursor.json"
cat >"$tmp/split-list.txt" <<'EOF'
crates/new/a.rs:1:1: replace a with b in n1
crates/old/a.rs:1:1: replace a with b in o1
crates/old/b.rs:2:1: replace a with b in o2
crates/old/c.rs:3:1: replace a with b in o3
EOF
cat >"$tmp/split-diff.txt" <<'EOF'
diff --git a/crates/new/a.rs b/crates/new/a.rs
--- a/crates/new/a.rs
+++ b/crates/new/a.rs
@@ -0,0 +1 @@
+fn new_line() {}
EOF
python3 "$PY" split --list "$tmp/split-list.txt" --diff "$tmp/split-diff.txt" \
  --cursor "$tmp/split-cursor.json" --out-new "$tmp/split-new.txt" --out-old "$tmp/split-old.txt" >/dev/null
assert_ok "split resumes the old queue at the saved name" \
  test "$(head -n 1 "$tmp/split-old.txt")" = "crates/old/c.rs:3:1: replace a with b in o3"
assert_ok "split keeps the new mutant on the new side" \
  test "$(cat "$tmp/split-new.txt")" = "crates/new/a.rs:1:1: replace a with b in n1"

python3 "$PY" advance-side --cursor "$tmp/split-cursor.json" --side old \
  --completed 1 --n-new 1 --n-old 3 --head def \
  --old-queue "$tmp/split-old.txt" --old-offset 1 \
  --elapsed 0 --budget 1000 --new-left 1 --old-left 2
assert_ok "advancing the backlog stores the next mutant name" \
  grep -q '"old_name": "crates/old/a.rs:1:1: replace a with b in o1"' "$tmp/split-cursor.json"

printf '%s\n' '{"new_base":"abc","new_skip":0,"old_index":0,"old_name":"","new_cap_used":false,"run_id":"7"}' >"$tmp/cap.json"
python3 "$PY" mark-cap --cursor "$tmp/cap.json" --elapsed 500 --budget 1000 --new-left 2 --old-left 2
assert_ok "half the budget with both sides left consumes the new cap" \
  grep -q '"new_cap_used": true' "$tmp/cap.json"
python3 "$PY" mark-cap --cursor "$tmp/cap.json" --elapsed 0 --budget 1000 --new-left 2 --old-left 2
assert_ok "a consumed new cap stays consumed for the rest of the night" \
  grep -q '"new_cap_used": true' "$tmp/cap.json"

NIGHTLY="$ROOT/scripts/mutants-nightly.sh"
TOML="$ROOT/.cargo/mutants.toml"
assert_ok "nightly list excludes rbitcoin-bench" \
  bash -c 'grep -q -- "--exclude '"'"'crates/rbitcoin-bench/\*\*/\*.rs'"'"' --list" "$1"' _ "$NIGHTLY"
assert_ok "nightly run excludes rbitcoin-bench" \
  bash -c 'grep -q -- "--exclude '"'"'crates/rbitcoin-bench/\*\*/\*.rs'"'"'" "$1"' _ "$NIGHTLY"
assert_ok "mutants.toml excludes rbitcoin-bench" \
  grep -q 'crates/rbitcoin-bench/\*\*/\*.rs' "$TOML"
WF="$ROOT/.github/workflows/mutants.yml"
CI="$ROOT/.github/workflows/ci.yml"
assert_ok "nightly cron is 00:47 UTC" \
  grep -q 'cron: "47 0 \* \* \*"' "$WF"
assert_ok "workflow_dispatch remains" \
  grep -q 'workflow_dispatch:' "$WF"
assert_ok "ci does not run the nightly mutants script" \
  bash -c '! grep -q mutants-nightly.sh "$1"' _ "$CI"
assert_ok "nightly uses the queue regex helper" \
  grep -q 'mutants_queue.py" re' "$NIGHTLY"
assert_ok "nightly examines one file" \
  grep -q -- '--file' "$NIGHTLY"
assert_ok "nightly batches by file" \
  grep -q 'mutants_queue.py" file-batch' "$NIGHTLY"
assert_ok "nightly asks which side the next batch is" \
  grep -q 'mutants_queue.py" next-side' "$NIGHTLY"
assert_ok "nightly persists the new cap" \
  grep -q 'mutants_queue.py" mark-cap' "$NIGHTLY"
assert_ok "nightly advances one side" \
  grep -q 'mutants_queue.py" advance-side' "$NIGHTLY"
assert_ok "nightly progress ignores extra outcomes" \
  grep -q 'mutants_queue.py" progress' "$NIGHTLY"
assert_ok "same-file cursor copy is skipped" \
  grep -q -- '-ef' "$NIGHTLY"
assert_ok "per-mutant timeout stays 20 minutes" \
  grep -q 'MUTANTS_TIMEOUT:-1200' "$NIGHTLY"
assert_ok "one job budget is 4 hours" \
  grep -q 'MUTANTS_BUDGET_SEC:-14400' "$NIGHTLY"
assert_ok "missed mutants do not fail the nightly script" \
  grep -q 'MISSED (not a failure' "$NIGHTLY"
assert_ok "queued nights are not cancelled" \
  grep -q 'cancel-in-progress: false' "$WF"

assert_ok "two jobs sum to 8 hours and each timeout has 30 minutes of slack under 6 hours" \
  python3 -c '
import pathlib, sys
text = pathlib.Path(sys.argv[1]).read_text()
budgets = text.count("MUTANTS_BUDGET_SEC: \"14400\"")
timeouts = text.count("timeout-minutes: 270")
assert budgets == 2, budgets
assert timeouts == 2, timeouts
assert 14400 * 2 == 8 * 3600
assert 270 <= 360
assert 270 >= 14400 // 60 + 30
' "$WF"

assert_ok "each mutant job restores the state branch before the queue and publishes it after" \
  python3 -c '
import pathlib, sys
lines = pathlib.Path(sys.argv[1]).read_text().splitlines()
starts = [i for i, line in enumerate(lines) if line.startswith("  mutants-")]
assert len(starts) == 2, starts
starts.append(len(lines))
for a, b in zip(starts, starts[1:]):
    block = lines[a:b]
    def at(needle):
        for i, line in enumerate(block):
            if needle in line:
                return i
        return -1
    restore, run, publish = at("restore-mutants-state.sh"), at("mutants-nightly.sh"), at("publish-mutants-state.sh")
    assert 0 <= restore < run < publish, (restore, run, publish)
' "$WF"

assert_ok "artifact download is not the only cursor restore" \
  grep -q 'git fetch' "$ROOT/scripts/restore-mutants-state.sh"
assert_ok "state branch is mutants-state" \
  grep -q 'mutants-state' "$ROOT/scripts/publish-mutants-state.sh"
assert_ok "publisher refuses master" \
  grep -q 'master|main' "$ROOT/scripts/publish-mutants-state.sh"
assert_ok "publisher pushes only the state branch ref" \
  grep -q 'HEAD:${BRANCH}' "$ROOT/scripts/publish-mutants-state.sh"
assert_ok "workflow names the state branch" \
  grep -q 'MUTANTS_STATE_BRANCH: mutants-state' "$WF"
assert_ok "both jobs share one run id" \
  bash -c 'test "$(grep -c "github.run_id" "$1")" -ge 2' _ "$WF"
assert_ok "state publish has contents write" \
  grep -q 'contents: write' "$WF"

dry="$(MUTANTS_STATE_DRY_RUN=1 "$ROOT/scripts/publish-mutants-state.sh")"
assert_ok "dry-run names the state branch and the cursor" \
  grep -q 'mutants-state/cursor.json' <<<"$dry"
assert_ok "dry-run names the miss-list update" \
  grep -q 'missed.txt' <<<"$dry"
assert_ok "dry-run does not push" \
  grep -q 'no push' <<<"$dry"
assert_ok "dry-run does not check out or push master" \
  bash -c '! printf "%s\n" "$1" | grep -E -q "checkout master|HEAD:master|push origin master"' _ "$dry"

set +e
MUTANTS_STATE_DRY_RUN=1 MUTANTS_STATE_BRANCH=master \
  "$ROOT/scripts/publish-mutants-state.sh" >/dev/null 2>"$tmp/refuse.err"
refuse_ec=$?
set -e
assert_ok "dry-run refuses to publish master" test "$refuse_ec" -ne 0

bare="$(mktemp -d)"
git init --bare -q "$bare"
printf '%s\n' '{"new_base":"abc","new_skip":1,"old_index":2,"old_name":"crates/old/a.rs:1:1: replace a with b in o1","new_cap_used":true,"run_id":"42"}' >"$tmp/pub-cursor.json"
printf '%s\n' 'MISSED crates/old/a.rs:1:1: replace a with b in o1' >"$tmp/pub-missed.txt"
MUTANTS_STATE_PUBLISH=1 MUTANTS_STATE_PUSH_URL="$bare" MUTANTS_STATE_BRANCH=mutants-state \
  MUTANTS_RUN_ID=42 MUTANTS_PHASE=1 \
  "$ROOT/scripts/publish-mutants-state.sh" "$tmp/pub-cursor.json" "$tmp/pub-missed.txt" \
  >"$tmp/pub1.out"
printf '%s\n' 'MISSED crates/old/b.rs:2:1: replace a with b in o2' >"$tmp/pub-missed-2.txt"
printf '%s\n' '{"new_base":"abc","new_skip":1,"old_index":3,"old_name":"crates/old/b.rs:2:1: replace a with b in o2","new_cap_used":true,"run_id":"42"}' >"$tmp/pub-cursor-2.json"
MUTANTS_STATE_PUBLISH=1 MUTANTS_STATE_PUSH_URL="$bare" MUTANTS_STATE_BRANCH=mutants-state \
  MUTANTS_RUN_ID=42 MUTANTS_PHASE=2 \
  "$ROOT/scripts/publish-mutants-state.sh" "$tmp/pub-cursor-2.json" "$tmp/pub-missed-2.txt" \
  >"$tmp/pub2.out"
pub_clone="$(mktemp -d)"
git clone -q --branch mutants-state "$bare" "$pub_clone"
assert_ok "published cursor is the second job cursor" \
  grep -E -q '"old_index": ?3' "$pub_clone/cursor.json"
assert_ok "miss lines from both jobs are on the branch" \
  bash -c 'grep -q "o1" "$1/missed.txt" && grep -q "o2" "$1/missed.txt"' _ "$pub_clone"
assert_ok "publish did not create master" \
  bash -c '! git --git-dir="$1" show-ref --verify --quiet refs/heads/master' _ "$bare"
assert_ok "publish created mutants-state" \
  bash -c 'git --git-dir="$1" show-ref --verify --quiet refs/heads/mutants-state' _ "$bare"

set +e
MUTANTS_STATE_PUBLISH=1 MUTANTS_STATE_PUSH_URL="$tmp/not-a-remote" MUTANTS_STATE_BRANCH=mutants-state \
  "$ROOT/scripts/publish-mutants-state.sh" "$tmp/pub-cursor.json" "$tmp/pub-missed.txt" \
  >"$tmp/bad-push.out" 2>"$tmp/bad-push.err"
bad_ec=$?
set -e
assert_ok "a failed state publish exits non-zero" test "$bad_ec" -ne 0

# Branch cursor wins over a disagreeing artifact. Local origin remote, no network.
src="$(mktemp -d)"
git init -q -b master "$src"
git -C "$src" config user.email "mutants-test@example.com"
git -C "$src" config user.name "mutants-test"
echo seed >"$src/README"
git -C "$src" add README
git -C "$src" commit -q -m seed
git -C "$src" remote add origin "$bare"
# The bare repo already has mutants-state from the publisher. Point a fresh
# branch cursor at a different name than the artifact by cloning that history
# and replacing the file is unnecessary: fetch the published branch.
printf '%s\n' '{"old_index":1,"old_name":"from-artifact"}' >"$tmp/artifact-cursor.json"
(
  cd "$src"
  MUTANTS_ARTIFACT_CURSOR="$tmp/artifact-cursor.json" \
    "$ROOT/scripts/restore-mutants-state.sh" "$tmp/restored.json"
)
assert_ok "restore prefers the state branch over the artifact" \
  grep -E -q '"old_index": ?3' "$tmp/restored.json"
assert_ok "restore does not keep the disagreeing artifact cursor" \
  bash -c '! grep -q from-artifact "$1"' _ "$tmp/restored.json"

git --git-dir="$bare" update-ref -d refs/heads/mutants-state
(
  cd "$src"
  MUTANTS_ARTIFACT_CURSOR="$tmp/artifact-cursor.json" \
    "$ROOT/scripts/restore-mutants-state.sh" "$tmp/restored-art.json"
)
assert_ok "missing state branch falls back to the artifact" \
  grep -q 'from-artifact' "$tmp/restored-art.json"
(
  cd "$src"
  MUTANTS_ARTIFACT_CURSOR="$tmp/no-artifact.json" \
    "$ROOT/scripts/restore-mutants-state.sh" "$tmp/restored-empty.json"
)
assert_ok "missing branch and artifact start from an empty cursor" \
  grep -E -q '"old_index": ?0' "$tmp/restored-empty.json"

echo "$PASS passed, $FAIL failed"
test "$FAIL" -eq 0
