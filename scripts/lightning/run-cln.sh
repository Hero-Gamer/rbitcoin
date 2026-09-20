#!/usr/bin/env bash
# Optional CLN smoke against rbitcoin-node. Skip 0 if lightningd is missing.
set -euo pipefail
if ! command -v lightningd >/dev/null 2>&1; then
  echo "skip: lightningd not on PATH"
  exit 0
fi
echo "run-cln: lightningd found; live fund/connect smoke not wired yet" >&2
exit 0
