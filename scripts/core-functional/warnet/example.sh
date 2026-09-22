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

"${COMPOSE[@]}" up -d --force-recreate --remove-orphans

cli() {
  local tank="$1"
  shift
  "${COMPOSE[@]}" exec -T "$tank" bitcoin-cli -regtest "$@"
}

height() {
  cli "$1" getblockcount 2>/dev/null | tr -d '[:space:]' || true
}

deadline=$((SECONDS + 90))
until [[ "$(height tank0)" == "0" && "$(height tank1)" == "0" ]]; do
  if (( SECONDS > deadline )); then
    echo "example.sh: tanks did not answer getblockcount" >&2
    "${COMPOSE[@]}" logs || true
    exit 1
  fi
  sleep 2
done

cli tank0 generate 1 >/dev/null

deadline=$((SECONDS + 60))
until [[ "$(height tank1)" != "" && "$(height tank1)" != "0" ]]; do
  if (( SECONDS > deadline )); then
    echo "example.sh: tank1 did not follow tank0 (height=$(height tank1))" >&2
    "${COMPOSE[@]}" logs || true
    exit 1
  fi
  sleep 2
done

echo "ok - tank0=$(height tank0) tank1=$(height tank1)"
