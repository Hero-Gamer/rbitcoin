#!/usr/bin/env bash
# Overlay functional gate: private Tor / i2pd / cjdns meshes, then cargo test.
# Default cargo test never calls this.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
cd "$ROOT"

LIST_NAMES=(
  tor_onion_two_node_v2
  tor_wallet_hs_electrum_esplora
  i2p_sam_two_node
  cjdns_tun_two_node
  ephemeral_socks_isolated_broadcast
)

if [[ "${1:-}" == "--list" ]]; then
  printf '%s\n' "${LIST_NAMES[@]}"
  exit 0
fi

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  echo "usage: $0 [--list] [cargo-test-filter...]"
  echo "starts private overlay daemons, then:"
  echo "  cargo test -p rbitcoin-test --features overlay --test overlay -- --quiet FILTER"
  exit 0
fi

if [[ -z "${OVERLAY_IN_NIX:-}" ]]; then
  if ! command -v nix >/dev/null 2>&1; then
    echo "overlay-functional: nix is required (flake overlayFunctional shell)" >&2
    exit 1
  fi
  exec nix develop "$ROOT#overlayFunctional" --command \
    env OVERLAY_IN_NIX=1 \
    CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target/dev}" \
    "$0" "$@"
fi

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "overlay-functional: missing $1 (use flake overlayFunctional)" >&2
    exit 1
  fi
}
need tor
need tor-gencert
need i2pd
need cjdroute
need python3
need cargo

WORKDIR="$(mktemp -d /tmp/rbitcoin-overlay.XXXXXX)"
cleanup() {
  local st=$?
  set +e
  if [[ -d "$WORKDIR/tor" ]]; then
    find "$WORKDIR/tor" -name tor.pid -print0 2>/dev/null \
      | xargs -0 -r -I{} sh -c 'kill "$(cat "$1")" 2>/dev/null' _ {}
  fi
  if [[ -d "$WORKDIR/i2p" ]]; then
    for p in "$WORKDIR/i2p"/n*/i2pd.pid; do
      [[ -f "$p" ]] && kill "$(cat "$p")" 2>/dev/null
    done
    if [[ -f "$WORKDIR/i2p/loopbacks" ]]; then
      while read -r ip; do
        sudo ip addr del "$ip/8" dev lo 2>/dev/null || true
      done <"$WORKDIR/i2p/loopbacks"
    fi
  fi
  if [[ -d "$WORKDIR/cjdns" ]]; then
    for p in "$WORKDIR/cjdns/a.pid" "$WORKDIR/cjdns/b.pid"; do
      [[ -f "$p" ]] && sudo kill "$(cat "$p")" 2>/dev/null
    done
    sudo pkill -f "cjdroute core $WORKDIR/cjdns" 2>/dev/null || true
    sudo ip link del rbtc0 2>/dev/null
    sudo ip link del rbtc1 2>/dev/null
  fi
  rm -rf "$WORKDIR"
  exit "$st"
}
trap cleanup EXIT INT TERM

echo "overlay-functional: workdir $WORKDIR" >&2

python3 "$HERE/tornet.py" "$WORKDIR/tor"
# shellcheck disable=SC1091
source "$WORKDIR/tor/env"
chmod +x "$HERE/i2pnet.sh" "$HERE/cjdns.sh"
"$HERE/i2pnet.sh" "$WORKDIR/i2p"
# shellcheck disable=SC1091
source "$WORKDIR/i2p/env"
"$HERE/cjdns.sh" "$WORKDIR/cjdns"
# shellcheck disable=SC1091
source "$WORKDIR/cjdns/env"

export OVERLAY_TOR_SOCKS OVERLAY_TOR_CONTROL OVERLAY_TOR_COOKIE
export OVERLAY_I2P_SAM OVERLAY_I2P_SAM_B
export OVERLAY_CJDNS_A OVERLAY_CJDNS_B

echo "overlay-functional: TOR_SOCKS=$OVERLAY_TOR_SOCKS TOR_CONTROL=$OVERLAY_TOR_CONTROL" >&2
echo "overlay-functional: I2P_SAM=$OVERLAY_I2P_SAM I2P_SAM_B=$OVERLAY_I2P_SAM_B" >&2
echo "overlay-functional: CJDNS_A=$OVERLAY_CJDNS_A CJDNS_B=$OVERLAY_CJDNS_B" >&2

cargo build -p rbitcoin-node
cargo test -p rbitcoin-test --features overlay --test overlay -- --quiet --test-threads=1 "$@"
echo "overlay-functional: journeys done"
