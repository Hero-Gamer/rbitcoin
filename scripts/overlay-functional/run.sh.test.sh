#!/usr/bin/env bash
# Contract pin for overlay-functional/run.sh (no daemons, no cargo).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
RUN="$ROOT/scripts/overlay-functional/run.sh"
PASS=0
FAIL=0

assert_ok() {
  local name="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    echo "ok - $name"
    PASS=$((PASS + 1))
  else
    echo "not ok - $name"
    FAIL=$((FAIL + 1))
  fi
}

assert_stdout() {
  local name="$1"
  local needle="$2"
  shift 2
  local out
  if ! out="$("$@" 2>/dev/null)"; then
    echo "not ok - $name (unexpected failure)"
    FAIL=$((FAIL + 1))
    return
  fi
  if printf '%s' "$out" | grep -q -- "$needle"; then
    echo "ok - $name"
    PASS=$((PASS + 1))
  else
    echo "not ok - $name (missing '$needle' in stdout: $out)"
    FAIL=$((FAIL + 1))
  fi
}

assert_stdout tor_list tor_onion_two_node_v2 "$RUN" --list
assert_stdout wallet_list tor_wallet_hs_electrum_esplora "$RUN" --list
assert_stdout i2p_list i2p_sam_two_node "$RUN" --list
assert_stdout cjdns_list cjdns_tun_two_node "$RUN" --list
assert_stdout eph_list ephemeral_socks_isolated_broadcast "$RUN" --list
assert_ok help "$RUN" --help

echo "$PASS passed, $FAIL failed"
[[ "$FAIL" -eq 0 ]]
