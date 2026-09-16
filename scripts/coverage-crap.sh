#!/usr/bin/env bash
# CRAP after a successful LCOV floor. Write absolute JSON. Gate is
# --fail-above --threshold 30; .cargo-crap.toml allowlists today's
# production offenders and excludes bench/tests. Never --fail-regression
# (llvm-cov coverage % jitters per function). Missing cargo-crap locally
# is skip; a gate trip is exit 1.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LCOV="${1:-$ROOT/coverage/lcov.info}"
OUT="${2:-$ROOT/coverage/crap.json}"
# -p not --workspace: workspace walks are crate-relative, so a
# **/rbitcoin-bench/** exclude never matches src/*.rs inside that crate.
# Skip rbitcoin-bench (optional host A/B) and rbitcoin-test.
PACKAGES=(
  rbitcoin-primitives
  rbitcoin-log
  rbitcoin-store
  rbitcoin-query
  rbitcoin-consensus
  rbitcoin-mempool
  rbitcoin-net
  rbitcoin-rpc
  rbitcoin-electrum
  rbitcoin-esplora
  rbitcoin-cli
  rbitcoin-node
)
CRAP_P=()
for p in "${PACKAGES[@]}"; do
  CRAP_P+=(-p "$p")
done

SUMMARY=(cargo crap "${CRAP_P[@]}" --lcov "$LCOV" --summary)
JSON=(cargo crap "${CRAP_P[@]}" --lcov "$LCOV" --format json --sort file --output "$OUT")
GATE=(cargo crap "${CRAP_P[@]}" --lcov "$LCOV" --fail-above --threshold 30 --summary)

if [[ "${CRAP_DRY_RUN:-}" == "1" ]]; then
  echo "${SUMMARY[*]}"
  echo "${JSON[*]}"
  echo "${GATE[*]}"
  exit 0
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "coverage-crap: skip (cargo not on PATH)"
  exit 0
fi
if ! command -v cargo-crap >/dev/null 2>&1 && ! cargo crap --help >/dev/null 2>&1; then
  echo "coverage-crap: skip (cargo-crap not installed)"
  exit 0
fi

if [[ ! -f "$LCOV" ]]; then
  echo "coverage-crap: skip (missing $LCOV)"
  exit 0
fi

mkdir -p "$(dirname "$OUT")"
if ! "${SUMMARY[@]}"; then
  echo "coverage-crap: summary failed" >&2
  exit 2
fi
if ! "${JSON[@]}"; then
  echo "coverage-crap: json failed" >&2
  exit 2
fi
echo "coverage-crap: fail-above 30 (allowlist in .cargo-crap.toml)"
if ! "${GATE[@]}"; then
  echo "coverage-crap: CRAP > 30 outside the allowlist" >&2
  exit 1
fi
exit 0
