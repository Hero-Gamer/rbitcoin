#!/usr/bin/env bash
# Text contract for the Warnet lab image (no docker required).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
PASS=0
FAIL=0

ok() { echo "ok - $1"; PASS=$((PASS + 1)); }
bad() { echo "not ok - $1"; FAIL=$((FAIL + 1)); }

EP="$HERE/entrypoint.sh"
DF="$HERE/Dockerfile"

if [[ -f "$EP" ]] && grep -q 'BITCOIN_DATA:-/root/.bitcoin' "$EP" \
  && grep -q 'exec' "$EP"; then
  ok "entrypoint defaults -datadir to BITCOIN_DATA or /root/.bitcoin"
else
  bad "entrypoint defaults -datadir to BITCOIN_DATA or /root/.bitcoin"
fi

if [[ -f "$DF" ]] \
  && grep -q 'rbitcoin-node' "$DF" \
  && grep -q 'bitcoind' "$DF" \
  && grep -q 'bitcoin-cli' "$DF" \
  && grep -q 'test_framework' "$DF" \
  && grep -q 'RBITCOIN_LOG_STDOUT' "$DF" \
  && grep -q 'RBITCOIN_LAB_HEAD_SCALE' "$DF"; then
  ok "Dockerfile copies node, shims, test_framework, lab env"
else
  bad "Dockerfile copies node, shims, test_framework, lab env"
fi

SHIM="$ROOT/scripts/core-functional/bitcoind"
if [[ -f "$SHIM" ]] \
  && grep -q 'RBITCOIN_LOG_STDOUT' "$SHIM" \
  && grep -q 'sys.stdout.buffer.write' "$SHIM"; then
  ok "shim tees node output when RBITCOIN_LOG_STDOUT is set"
else
  bad "shim tees node output when RBITCOIN_LOG_STDOUT is set"
fi

DOC="$ROOT/docs/core-functional.md"
KIND_OUT=""
if KIND_OUT="$(python3 - "$DOC" <<'PY'
import re
import sys
from pathlib import Path

doc = Path(sys.argv[1]).read_text()
start = doc.find("### Operator kind")
if start < 0:
    sys.exit("missing operator kind section")
section = doc[start:]
semver = re.compile(r"^\d+\.\d+\.\d+(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$")
found = False
for line in section.splitlines():
    if "docker build -t" not in line and "kind load docker-image" not in line:
        continue
    found = True
    if ":local" in line or "rbitcoin-warnet:local" in line:
        sys.exit(f"kind instructions still use local: {line}")
    match = re.search(r"rbitcoin-warnet:([^\s\\]+)", line)
    if match is None or semver.fullmatch(match.group(1)) is None:
        sys.exit(f"kind tag is not semver: {line}")
if not found:
    sys.exit("kind section has no docker build -t or kind load docker-image")
PY
)"; then
  ok "kind instructions use a semver image tag"
else
  bad "kind instructions use a semver image tag (${KIND_OUT})"
fi

echo
echo "$PASS passed, $FAIL failed"
if [[ "$FAIL" -ne 0 ]]; then
  exit 1
fi
