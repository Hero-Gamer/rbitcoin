#!/usr/bin/env bash
# Model-checked concurrency. loom explores claim order, the UTXO view stamp,
# the Class C sequence counter, and Class C flush exclusion. shuttle runs
# the script-pool idle turn.
# The engine is a `--cfg`, not a feature, so each engine builds in its own
# target dir (TESTING.md, Artifact silos).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

engine="${1:-}"
case "$engine" in
  loom | shuttle) shift ;;
  *)
    echo "usage: $0 <loom|shuttle> [extra cargo test args...]" >&2
    exit 2
    ;;
esac

RUSTFLAGS="${RUSTFLAGS:-} --cfg $engine"
export RUSTFLAGS="${RUSTFLAGS# }"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR_MODEL:-$ROOT/target/model-$engine}"
cmd=(cargo test -p rbitcoin-store -p rbitcoin-consensus --test model "$@")

exec "${cmd[@]}"
