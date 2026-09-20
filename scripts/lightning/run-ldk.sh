#!/usr/bin/env bash
# Optional ldk-node Esplora smoke. Skip 0 if ldk-node is not a cargo example here.
set -euo pipefail
if ! command -v ldk-node >/dev/null 2>&1; then
  echo "skip: ldk-node not on PATH"
  exit 0
fi
echo "run-ldk: ldk-node found; live sync smoke not wired yet" >&2
exit 0
