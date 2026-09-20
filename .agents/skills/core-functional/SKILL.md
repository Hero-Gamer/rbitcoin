---
name: core-functional
description: >-
  Run or fix the Bitcoin Core functional harness. Use when a PR is labeled
  core-functional, a ship version-bump needs that job, or
  scripts/core-functional is red. Do not treat this harness as the default
  PR pin.
---

# Core functional harness

Owner: [`docs/core-functional.md`](../../../docs/core-functional.md).
Default `cargo test` is the PR pin ([`TESTING.md`](../../../TESTING.md)).
Do not label ordinary net or RPC PRs. Label harness changes and every ship
version-bump PR.

When the labeled job is red, reproduce locally. Do not push and wait on
GitHub Actions as the inner loop. Rebuild `rbitcoin-node` after each product
change.

```bash
# Keep CARGO_TARGET_DIR if already set.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target/dev}"
cargo build -p rbitcoin-node
RBITCOIN_NODE="$CARGO_TARGET_DIR/debug/rbitcoin-node" \
  ./scripts/core-functional/run.sh <failing.py>
```

Inventory and runner details, including what stays `skip`, live in
`docs/core-functional.md`. In-tree Red for the same contract is still a
catalog journey or unit; this script is the labeled-job oracle.
