#!/usr/bin/env bash
# CRAP after a successful LCOV gate. Always write absolute JSON (report-v1,
# --sort file) for the crap-report artifact / baseline refresh. When
# crap_baseline.json exists, a second invocation --fail-regression (Q-55).
# Never --fail-above (CRAP equals CC at ≥90% lines). Missing cargo-crap
# locally is skip; a gate trip is exit 1.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LCOV="${1:-$ROOT/coverage/lcov.info}"
OUT="${2:-$ROOT/coverage/crap.json}"
BASE="${CRAP_BASELINE:-$ROOT/crap_baseline.json}"

SUMMARY=(cargo crap --workspace --lcov "$LCOV" --summary)
JSON=(cargo crap --workspace --lcov "$LCOV" --format json --sort file --output "$OUT")
GATE=()
if [[ -f "$BASE" ]]; then
  GATE=(cargo crap --workspace --lcov "$LCOV" --baseline "$BASE" --fail-regression --summary)
fi

if [[ "${CRAP_DRY_RUN:-}" == "1" ]]; then
  echo "${SUMMARY[*]}"
  echo "${JSON[*]}"
  if [[ ${#GATE[@]} -gt 0 ]]; then
    echo "${GATE[*]}"
  fi
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
if [[ ${#GATE[@]} -eq 0 ]]; then
  echo "coverage-crap: no baseline (report-only)"
  exit 0
fi
echo "coverage-crap: fail-regression vs $BASE"
if ! "${GATE[@]}"; then
  echo "coverage-crap: CRAP regression vs $BASE" >&2
  exit 1
fi
exit 0
