#!/usr/bin/env bash
# Two-tank regtest on the Warnet lab image. Needs Docker. Not kind/Helm.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
HERE="$(cd "$(dirname "$0")" && pwd)"
NODE_BIN="${NODE_BIN:-$ROOT/target/dev/debug/rbitcoin-node}"
IMAGE="${IMAGE:-rbitcoin-warnet:local}"
COMPOSE=(docker compose -f "$HERE/example-compose.yml" -p rbitcoin-warnet-example)

if ! command -v docker >/dev/null 2>&1; then
  echo "example.sh: docker is required" >&2
  exit 1
fi
if [[ ! -x "$NODE_BIN" ]]; then
  echo "example.sh: missing node binary $NODE_BIN" >&2
  exit 1
fi
if ldd "$NODE_BIN" 2>/dev/null | grep -q '/nix/store/'; then
  echo "example.sh: $NODE_BIN links the nix dynamic loader. Build it with rust:1.95.0-bookworm (see the Dockerfile comment)." >&2
  exit 1
fi

docker build -t "$IMAGE" \
  --build-arg "NODE_BIN=${NODE_BIN#"$ROOT"/}" \
  -f "$HERE/Dockerfile" \
  "$ROOT"

cleanup() {
  "${COMPOSE[@]}" down -t 5 >/dev/null 2>&1 || true
}
trap cleanup EXIT

# Genesis --connect enters tip-follow, so tank1 answers RPC while tank0 is
# down. The 2s redial is what connects once that hostname exists.
"${COMPOSE[@]}" up -d --force-recreate --remove-orphans tank1

cli() {
  local tank="$1"
  shift
  "${COMPOSE[@]}" exec -T "$tank" bitcoin-cli -regtest "$@"
}

height() {
  cli "$1" getblockcount 2>/dev/null | tr -d '[:space:]' || true
}

peers() {
  cli "$1" getconnectioncount 2>/dev/null | tr -d '[:space:]' || true
}

# bitcoin-cli uses json.dumps, which writes `"addr": "ip:port"`.
peer_addr() {
  local raw
  raw="$(cli "$1" getpeerinfo 2>/dev/null || true)"
  if [[ "$raw" =~ \"addr\":[[:space:]]*\"([^\"]+)\" ]]; then
    printf '%s\n' "${BASH_REMATCH[1]}"
  fi
}

fail() {
  echo "example.sh: $*" >&2
  "${COMPOSE[@]}" ps -a >&2 || true
  "${COMPOSE[@]}" logs --no-color >&2 || true
  exit 1
}

deadline=$((SECONDS + 90))
until [[ "$(height tank1)" == "0" ]]; do
  if (( SECONDS > deadline )); then
    fail "tank1 did not answer getblockcount (got '$(height tank1)')"
  fi
  sleep 2
done

if [[ "$(peers tank1)" != "0" ]]; then
  fail "tank1 had a peer before tank0 existed (peers='$(peers tank1)' addr='$(peer_addr tank1)')"
fi

"${COMPOSE[@]}" up -d --remove-orphans tank0

deadline=$((SECONDS + 90))
until [[ "$(height tank0)" == "0" ]]; do
  if (( SECONDS > deadline )); then
    fail "tank0 did not answer getblockcount (got '$(height tank0)')"
  fi
  sleep 2
done

cli tank0 generate 1 >/dev/null
want="$(height tank0)"
if [[ "$want" == "" || "$want" == "0" ]]; then
  fail "tank0 generate did not advance (height='$want')"
fi

deadline=$((SECONDS + 90))
addr=""
while (( SECONDS <= deadline )); do
  a="$(peer_addr tank1)"
  if [[ "$(height tank1)" == "$want" && "$(peers tank1)" != "" && "$(peers tank1)" != "0" ]]; then
    case "$a" in
      ""|0.0.0.0:*) ;;
      *) addr="$a"; break ;;
    esac
  fi
  sleep 2
done
if [[ -z "$addr" ]]; then
  fail "tank1 did not follow tank0 (height='$(height tank1)' want='$want' peers='$(peers tank1)' addr='$(peer_addr tank1)')"
fi

echo "ok - tank0=$want tank1=$(height tank1) peers=$(peers tank1) addr=$addr"
