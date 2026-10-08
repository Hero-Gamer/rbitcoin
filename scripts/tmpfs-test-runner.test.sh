#!/usr/bin/env bash
# Contract: the runner gives the child a private TMPDIR under the base,
# passes the exit code through, removes the dir, and falls back to the
# caller's TMPDIR when the base is missing or short on space. The child
# also inherits an address-space cap so one allocation cannot exhaust the
# hosted runner.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RUN="$ROOT/scripts/tmpfs-test-runner.sh"
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

WORK="$(mktemp -d "${TMPDIR:-/tmp}/rbtc-tmpfs-runner-test.XXXXXX")"
trap 'rm -rf -- "$WORK"' EXIT
BASE="$WORK/base"
CALLER="$WORK/caller"
mkdir -p "$BASE" "$CALLER"

# Child writes a file into its TMPDIR, records the path, exits 3.
child='touch "$TMPDIR/x" && printf %s "$TMPDIR" >"$1" && exit 3'

rc=0
RBTC_TEST_TMPFS="$BASE" RBTC_TEST_TMPFS_MIN_MB=0 TMPDIR="$CALLER" \
  "$RUN" bash -c "$child" _ "$WORK/seen" || rc=$?
seen="$(cat "$WORK/seen")"
assert_ok "exit code passes through" test "$rc" -eq 3
assert_ok "TMPDIR is a private dir under the base" \
  bash -c '[[ "$1" == "$2"/rbtc-test.* ]]' _ "$seen" "$BASE"
assert_ok "private dir is removed after the child" test ! -e "$seen"
assert_ok "base is left empty" test -z "$(ls -A "$BASE")"

rc=0
RBTC_TEST_TMPFS="$WORK/missing" TMPDIR="$CALLER" \
  "$RUN" bash -c "$child" _ "$WORK/seen" || rc=$?
assert_ok "missing base keeps the caller TMPDIR" test "$(cat "$WORK/seen")" = "$CALLER"
assert_ok "missing base still passes the exit code" test "$rc" -eq 3

rc=0
RBTC_TEST_TMPFS="$BASE" RBTC_TEST_TMPFS_MIN_MB=999999999 TMPDIR="$CALLER" \
  "$RUN" bash -c "$child" _ "$WORK/seen" || rc=$?
assert_ok "short base keeps the caller TMPDIR" test "$(cat "$WORK/seen")" = "$CALLER"

# A small heap fits. A heap past the cap must fail inside the child.
rc=0
RBTC_TEST_AS_MB=512 RBTC_TEST_TMPFS="$BASE" RBTC_TEST_TMPFS_MIN_MB=0 \
  "$RUN" python3 -c 'b = bytearray(32 * 1024 * 1024); b[-1] = 1' || rc=$?
assert_ok "allocation under the address-space cap succeeds" test "$rc" -eq 0

rc=0
RBTC_TEST_AS_MB=128 RBTC_TEST_TMPFS="$BASE" RBTC_TEST_TMPFS_MIN_MB=0 \
  "$RUN" python3 -c 'b = bytearray(400 * 1024 * 1024); b[-1] = 1' >/dev/null 2>&1 || rc=$?
assert_ok "allocation past the address-space cap fails" test "$rc" -ne 0

rc=0
RBTC_TEST_AS_MB=0 RBTC_TEST_TMPFS="$BASE" RBTC_TEST_TMPFS_MIN_MB=0 \
  "$RUN" python3 -c 'b = bytearray(200 * 1024 * 1024); b[-1] = 1' || rc=$?
assert_ok "address-space cap of 0 is unlimited" test "$rc" -eq 0

assert_ok "default address-space cap is 6 GiB" \
  grep -q 'RBTC_TEST_AS_MB:-6144' "$RUN"

echo "tmpfs-test-runner: $PASS passed, $FAIL failed"
((FAIL == 0))
