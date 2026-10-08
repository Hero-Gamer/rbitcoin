#!/usr/bin/env bash
# Cargo test runner: each test binary gets a private TMPDIR on tmpfs, so
# store fsyncs cost nothing and fixtures never touch the runner disk.
# The binary also inherits an address-space cap, so one mutant that grows
# a buffer dies in this process instead of taking down the hosted runner.
#
#   CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER=$PWD/scripts/tmpfs-test-runner.sh
#
# Falls back to the caller's TMPDIR when the base is missing, not writable,
# or has less than RBTC_TEST_TMPFS_MIN_MB free. RBTC_TEST_AS_MB=0 leaves
# the caller's address-space limit. Owner: TESTING.md.
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

# KiB. 6 GiB is under a 16 GiB hosted runner and above a real workspace
# test. ulimit -v counts address space, which a growing Vec fills.
as_mb="${RBTC_TEST_AS_MB:-6144}"
if [[ "$as_mb" != 0 ]]; then
  ulimit -v $((as_mb * 1024)) || exit 125
fi

"$@"
