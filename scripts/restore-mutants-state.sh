#!/usr/bin/env bash
# Copy the mutants cursor from the state branch. That branch wins when an
# artifact cursor is also present and they disagree. A missing branch
# falls back to the artifact, then to an empty cursor.
# Owner: TESTING.md (Mutation testing).
set -euo pipefail

dest="${1:?cursor dest}"
branch="${MUTANTS_STATE_BRANCH:-mutants-state}"
remote="${MUTANTS_STATE_REMOTE:-origin}"

case "$branch" in
  master|main)
    echo "restore-mutants-state: refuse ${branch}" >&2
    exit 1
    ;;
esac

mkdir -p "$(dirname "$dest")"

ref=""
if GIT_TERMINAL_PROMPT=0 git fetch "$remote" \
  "refs/heads/${branch}:refs/remotes/${remote}/${branch}" >/dev/null 2>&1; then
  ref="refs/remotes/${remote}/${branch}"
elif git show-ref --verify --quiet "refs/heads/${branch}"; then
  ref="refs/heads/${branch}"
fi

if [[ -n "$ref" ]] && git cat-file -e "${ref}:cursor.json" 2>/dev/null; then
  git show "${ref}:cursor.json" >"$dest"
  echo "restore-mutants-state: cursor from ${branch}"
elif [[ -n "${MUTANTS_ARTIFACT_CURSOR:-}" && -f "${MUTANTS_ARTIFACT_CURSOR}" ]]; then
  cp "${MUTANTS_ARTIFACT_CURSOR}" "$dest"
  echo "restore-mutants-state: cursor from artifact"
else
  printf '%s\n' '{"new_base":"","new_skip":0,"old_index":0,"old_name":"","new_cap_used":false,"run_id":""}' >"$dest"
  echo "restore-mutants-state: empty cursor"
fi
