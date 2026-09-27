#!/usr/bin/env bash
# Cargo test runner: each test binary gets a private TMPDIR on tmpfs, so
# store fsyncs cost nothing and fixtures never touch the runner disk.
#
#   CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER=$PWD/scripts/tmpfs-test-runner.sh
#
# Falls back to the caller's TMPDIR when the base is missing, not writable,
# or has less than RBTC_TEST_TMPFS_MIN_MB free. Owner: TESTING.md.
set -uo pipefail

base="${RBTC_TEST_TMPFS:-/dev/shm}"
min_mb="${RBTC_TEST_TMPFS_MIN_MB:-2048}"
dir=""

if [[ -d "$base" && -w "$base" ]]; then
  free_kb="$(df -Pk "$base" 2>/dev/null | awk 'NR == 2 { print $4 }')"
  if [[ -n "$free_kb" ]] && ((free_kb >= min_mb * 1024)); then
    dir="$(mktemp -d "$base/rbtc-test.XXXXXX" 2>/dev/null)" || dir=""
  fi
fi

if [[ -n "$dir" ]]; then
  trap 'rm -rf -- "$dir"' EXIT
  export TMPDIR="$dir"
fi

"$@"
