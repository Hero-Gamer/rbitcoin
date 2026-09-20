#!/usr/bin/env bash
# Contract: Core bitcoin-cli argv (-datadir=, -rpcport, …) reaches rbitcoin-cli
# as --datadir. Does not invoke a node.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
WRAP="$ROOT/scripts/lightning/bitcoin-cli"
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

stub="$(mktemp)"
trap 'rm -f "$stub" "$stub.out"' EXIT
cat >"$stub" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$@" >"${0}.out"
exit 0
STUB
chmod +x "$stub"

out="$(RBITCOIN_CLI="$stub" "$WRAP" -datadir=/tmp/rbtc-ln-datadir getblockchaininfo)" || true
assert_ok "wrapper exists and is executable" test -x "$WRAP"
assert_ok "forwards getblockchaininfo" grep -qx "getblockchaininfo" "$stub.out"
assert_ok "maps -datadir= to --datadir" grep -qx -- "--datadir" "$stub.out"
assert_ok "datadir path follows --datadir" grep -qx "/tmp/rbtc-ln-datadir" "$stub.out"

RBITCOIN_CLI="$stub" "$WRAP" --datadir /tmp/other -rpcport=8332 -rpcconnect=127.0.0.1 estimatesmartfee 6 >/dev/null
assert_ok "forwards estimatesmartfee" grep -qx "estimatesmartfee" "$stub.out"
assert_ok "forwards conf target" grep -qx "6" "$stub.out"
assert_ok "drops -rpcport" bash -c '! grep -q rpcport "$1"' _ "$stub.out"
assert_ok "drops -rpcconnect" bash -c '! grep -q rpcconnect "$1"' _ "$stub.out"

if [[ "$FAIL" -ne 0 ]]; then
  echo "bitcoin-cli.test.sh: $PASS passed, $FAIL failed"
  exit 1
fi
echo "bitcoin-cli.test.sh: $PASS passed"
