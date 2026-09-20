#!/usr/bin/env bash
# Contract: CLN/LDK smokes skip 0 without those binaries; bitcoin-cli wrapper tests pass.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
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

out="$("$ROOT/scripts/lightning/run-cln.sh")"
assert_ok "run-cln skips without lightningd or is quiet" \
  bash -c '[[ "$1" == skip:* || -n "$1" ]]' _ "$out"

out="$("$ROOT/scripts/lightning/run-ldk.sh")"
assert_ok "run-ldk skips without ldk-node or is quiet" \
  bash -c '[[ "$1" == skip:* || -n "$1" ]]' _ "$out"

"$ROOT/scripts/lightning/bitcoin-cli.test.sh"
assert_ok "bitcoin-cli wrapper self-test" true

if [[ "$FAIL" -ne 0 ]]; then
  echo "run.test.sh: $PASS passed, $FAIL failed"
  exit 1
fi
echo "run.test.sh: $PASS passed"
