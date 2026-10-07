#!/usr/bin/env bash
# Time-boxed workspace mutants. New lines since the cursor first, then a
# rotating backlog. While both queues still have work, new batches run
# until a quarter of this job's budget has elapsed, then the backlog.
# A later invocation in the same night does not open another new window.
# MISSED is written for humans; it does not fail the run.
# Owner: TESTING.md (Mutation testing).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# 5 hours. One workspace suite is the unit of work, and two of them in
# parallel do not fit a hosted runner, so this is -j 1. The job timeout
# is 30 minutes longer so the script can stop itself and the workflow can
# publish the state branch. GitHub-hosted jobs cannot run longer than 6 hours.
BUDGET_SEC="${MUTANTS_BUDGET_SEC:-18000}"
# One cargo-mutants process per source file. 27.1.0 emits `..` struct
# field deletes without applying --re, so a workspace-wide --re batch
# retests every such delete. --file keeps that repeat inside this file.
# The cap is how many queued names from that file one process may take.
BATCH="${MUTANTS_BATCH:-200}"
# A mutant that has not finished in 20 minutes is a hang, not a miss.
MUTANT_TIMEOUT="${MUTANTS_TIMEOUT:-1200}"
CURSOR="${MUTANTS_CURSOR:-mutants-nightly/cursor.json}"
OUT="${MUTANTS_OUT:-mutants-nightly}"

mkdir -p "$OUT"
if [[ ! -f "$CURSOR" ]]; then
  printf '%s\n' '{"new_base":"","new_skip":0,"old_index":0,"old_name":"","new_cap_used":false,"run_id":""}' >"$CURSOR"
fi
python3 "$ROOT/scripts/mutants_queue.py" night --cursor "$CURSOR" --run-id "${MUTANTS_RUN_ID:-}"

new_base="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("new_base",""))' "$CURSOR")"
new_skip="$(python3 -c 'import json,sys; print(int(json.load(open(sys.argv[1])).get("new_skip",0)))' "$CURSOR")"
old_index="$(python3 -c 'import json,sys; print(int(json.load(open(sys.argv[1])).get("old_index",0)))' "$CURSOR")"

if [[ -z "$new_base" ]] || ! git rev-parse --verify "${new_base}^{commit}" >/dev/null 2>&1; then
  new_base="$(git rev-list -n 1 --before="24 hours ago" HEAD || true)"
  if [[ -z "$new_base" ]]; then
    new_base="$(git rev-parse HEAD)"
  fi
  new_skip=0
  python3 - "$CURSOR" "$new_base" <<'PY'
import json, sys
path, base = sys.argv[1], sys.argv[2]
cur = json.load(open(path))
cur["new_base"] = base
cur["new_skip"] = 0
json.dump(cur, open(path, "w"), indent=2)
open(path, "a").write("\n")
PY
fi

head_sha="$(git rev-parse HEAD)"
git diff "$new_base"..HEAD --unified=0 -- crates >"$OUT/new.diff" || true

echo "mutants-nightly: listing workspace mutants"
# rbitcoin-bench is an optional host client, not a mutants gate.
# CLI --exclude replaces exclude_globs in .cargo/mutants.toml (same glob).
cargo mutants --workspace --exclude 'crates/rbitcoin-bench/**/*.rs' --list >"$OUT/all.txt"

read -r n_new n_old < <(python3 "$ROOT/scripts/mutants_queue.py" split \
  --list "$OUT/all.txt" --diff "$OUT/new.diff" \
  --cursor "$CURSOR" --head "$head_sha" \
  --out-new "$OUT/new.txt" --out-old "$OUT/old.txt")
new_skip="$(python3 "$ROOT/scripts/mutants_queue.py" show --cursor "$CURSOR" --key new_skip)"
old_index="$(python3 "$ROOT/scripts/mutants_queue.py" show --cursor "$CURSOR" --key old_index)"
new_cap="$(python3 "$ROOT/scripts/mutants_queue.py" show --cursor "$CURSOR" --key new_cap_used)"

echo "mutants-nightly: new=$n_new skip=$new_skip old=$n_old index=$old_index budget=${BUDGET_SEC}s new_cap=$new_cap"

: >"$OUT/missed.txt"
: >"$OUT/ran.txt"
completed_total=0

new_len="$(grep -c . "$OUT/new.txt" || true)"
old_len="$(grep -c . "$OUT/old.txt" || true)"
new_off=0
old_off=0
start=$SECONDS
deadline=$((start + BUDGET_SEC))
while ((SECONDS < deadline)); do
  remain=$((deadline - SECONDS))
  if ((remain < 60)); then
    break
  fi
  elapsed=$((SECONDS - start))
  new_left=$((new_len - new_off))
  old_left=$((old_len - old_off))
  python3 "$ROOT/scripts/mutants_queue.py" mark-cap \
    --cursor "$CURSOR" --elapsed "$elapsed" --budget "$BUDGET_SEC" \
    --new-left "$new_left" --old-left "$old_left"
  new_cap="$(python3 "$ROOT/scripts/mutants_queue.py" show --cursor "$CURSOR" --key new_cap_used)"
  side="$(python3 "$ROOT/scripts/mutants_queue.py" next-side \
    --elapsed "$elapsed" --budget "$BUDGET_SEC" \
    --new-left "$new_left" --old-left "$old_left" \
    --new-cap-used "$new_cap")"
  if [[ -z "$side" ]]; then
    break
  fi
  if [[ "$side" == "new" ]]; then
    queue="$OUT/new.txt"
    offset=$new_off
  else
    queue="$OUT/old.txt"
    offset=$old_off
  fi
  slice="$OUT/batch-$side-$offset.names"
  python3 "$ROOT/scripts/mutants_queue.py" file-batch \
    --queue "$queue" --offset "$offset" --cap "$BATCH" >"$slice"
  requested="$(grep -c . "$slice" || true)"
  if ((requested == 0)); then
    break
  fi
  file="$(head -n 1 "$slice" | cut -d: -f1)"
  RE_ARGS=(--file "$file")
  while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    escaped="$(python3 "$ROOT/scripts/mutants_queue.py" re "$line")"
    RE_ARGS+=(--re "$escaped")
  done <"$slice"
  echo "mutants-nightly: batch side=$side at $offset ($requested mutants in $file, ${remain}s left)"
  set +e
  timeout --signal=TERM --kill-after=60s "$remain" \
    cargo mutants --workspace --exclude 'crates/rbitcoin-bench/**/*.rs' \
      --test-workspace=true --baseline=skip --caught --unviable \
      -j 1 --timeout "$MUTANT_TIMEOUT" \
      "${RE_ARGS[@]}" \
      >"$OUT/batch-$side-$offset.log" 2>&1
  ec=$?
  set -e
  grep -E '^MISSED' "$OUT/batch-$side-$offset.log" >>"$OUT/missed.txt" || true
  # A clean exit consumed the slice. A killed process counts only the
  # requested names that printed an outcome, in order. Unfiltered
  # struct-field deletes in this file do not advance the cursor.
  if ((ec == 0)); then
    step=$requested
  else
    step="$(python3 "$ROOT/scripts/mutants_queue.py" progress --log "$OUT/batch-$side-$offset.log" --slice "$slice")"
    if ((step > requested)); then
      step=$requested
    fi
    if ((step == 0)); then
      echo "mutants-nightly: batch made no progress (exit $ec); same queue next night"
      break
    fi
  fi
  if [[ "$side" == "new" ]]; then
    new_off=$((new_off + step))
  else
    old_off=$((old_off + step))
  fi
  now=$((SECONDS - start))
  python3 "$ROOT/scripts/mutants_queue.py" advance-side \
    --cursor "$CURSOR" --side "$side" --completed "$step" \
    --n-new "$n_new" --n-old "$n_old" --head "$head_sha" \
    --old-queue "$OUT/old.txt" --old-offset "$old_off" \
    --elapsed "$now" --budget "$BUDGET_SEC" \
    --new-left "$((new_len - new_off))" --old-left "$((old_len - old_off))"
  completed_total=$((completed_total + step))
  if ((ec == 124)); then
    echo "mutants-nightly: budget exhausted after $completed_total mutants"
    break
  fi
done

python3 "$ROOT/scripts/mutants_queue.py" mark-cap \
  --cursor "$CURSOR" --elapsed "$((SECONDS - start))" --budget "$BUDGET_SEC" \
  --new-left "$((new_len - new_off))" --old-left "$((old_len - old_off))"

# The default cursor path already lives in the artifact directory.
if [[ ! "$CURSOR" -ef "$OUT/cursor.json" ]]; then
  cp "$CURSOR" "$OUT/cursor.json"
fi
echo "mutants-nightly: completed=$completed_total missed=$(grep -c . "$OUT/missed.txt" || true)"
if [[ -s "$OUT/missed.txt" ]]; then
  echo "mutants-nightly: MISSED (not a failure; extend a journey or delete the expression)"
  cat "$OUT/missed.txt"
fi
