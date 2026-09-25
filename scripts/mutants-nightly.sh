#!/usr/bin/env bash
# Time-boxed workspace mutants. New lines since the cursor first, then a
# rotating backlog. MISSED is written for humans; it does not fail the run.
# Owner: TESTING.md (Mutation testing).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# 90 minutes. One workspace suite is the unit of work, and two of them in
# parallel do not fit a hosted runner, so this is -j 1. An hour finishes
# only a couple of mutants after setup; 90 minutes can clear a normal day's
# new mutants and still walk the backlog.
BUDGET_SEC="${MUTANTS_BUDGET_SEC:-5400}"
BATCH="${MUTANTS_BATCH:-4}"
# A mutant that has not finished in 20 minutes is a hang, not a miss.
MUTANT_TIMEOUT="${MUTANTS_TIMEOUT:-1200}"
CURSOR="${MUTANTS_CURSOR:-mutants-nightly/cursor.json}"
OUT="${MUTANTS_OUT:-mutants-nightly}"

mkdir -p "$OUT"
if [[ ! -f "$CURSOR" ]]; then
  printf '%s\n' '{"new_base":"","new_skip":0,"old_index":0}' >"$CURSOR"
fi

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

python3 "$ROOT/scripts/mutants_queue.py" order \
  --list "$OUT/all.txt" --diff "$OUT/new.diff" \
  --old-index "$old_index" --new-skip "$new_skip" >"$OUT/queue.txt"

read -r n_new n_old < <(python3 - "$OUT/all.txt" "$OUT/new.diff" <<'PY'
import sys
sys.path.insert(0, "scripts")
from mutants_queue import changed_lines, load_mutants, parse_mutant
mutants = load_mutants(open(sys.argv[1]).read())
changed = changed_lines(open(sys.argv[2]).read())
n_new = sum(1 for line in mutants if (p := parse_mutant(line)) and (p[0], p[1]) in changed)
print(n_new, len(mutants) - n_new)
PY
)

echo "mutants-nightly: new=$n_new skip=$new_skip old=$n_old index=$old_index budget=${BUDGET_SEC}s"

deadline=$((SECONDS + BUDGET_SEC))
: >"$OUT/missed.txt"
: >"$OUT/ran.txt"
completed_total=0

re_args_for_batch() {
  local start="$1" count="$2" i line
  RE_ARGS=()
  i=0
  while IFS= read -r line; do
    if ((i >= start && i < start + count)); then
      local escaped
      escaped="$(python3 -c 'import re,sys; print(re.escape(sys.argv[1]))' "$line")"
      RE_ARGS+=(--re "$escaped")
    fi
    i=$((i + 1))
  done <"$OUT/queue.txt"
}

queue_len="$(grep -c . "$OUT/queue.txt" || true)"
offset=0
while ((offset < queue_len && SECONDS < deadline)); do
  remain=$((deadline - SECONDS))
  if ((remain < 60)); then
    break
  fi
  re_args_for_batch "$offset" "$BATCH"
  if ((${#RE_ARGS[@]} == 0)); then
    break
  fi
  echo "mutants-nightly: batch at $offset (${#RE_ARGS[@]} regexes, ${remain}s left)"
  set +e
  timeout --signal=TERM --kill-after=60s "$remain" \
    cargo mutants --workspace --exclude 'crates/rbitcoin-bench/**/*.rs' \
      --test-workspace=true --baseline=skip \
      -j 1 --timeout "$MUTANT_TIMEOUT" \
      "${RE_ARGS[@]}" \
      >"$OUT/batch-$offset.log" 2>&1
  ec=$?
  set -e
  # Count finished scenarios. A killed batch still advances through those.
  done_n="$(grep -cE '^(caught|MISSED|TIMEOUT|unviable)' "$OUT/batch-$offset.log" || true)"
  grep -E '^MISSED' "$OUT/batch-$offset.log" >>"$OUT/missed.txt" || true
  if ((done_n == 0)); then
    echo "mutants-nightly: batch made no progress (exit $ec); same queue next night"
    break
  fi
  python3 "$ROOT/scripts/mutants_queue.py" advance \
    --cursor "$CURSOR" --n-new "$n_new" --n-old "$n_old" \
    --completed "$done_n" --head "$head_sha"
  completed_total=$((completed_total + done_n))
  offset=$((offset + done_n))
  if ((ec == 124)); then
    echo "mutants-nightly: budget exhausted after $completed_total mutants"
    break
  fi
done

cp "$CURSOR" "$OUT/cursor.json"
echo "mutants-nightly: completed=$completed_total missed=$(grep -c . "$OUT/missed.txt" || true)"
if [[ -s "$OUT/missed.txt" ]]; then
  echo "mutants-nightly: MISSED (not a failure; extend a journey or keep a guts unit)"
  cat "$OUT/missed.txt"
fi
