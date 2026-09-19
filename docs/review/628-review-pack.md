# Review pack for #628 - Hero-Gamer/rbitcoin

Fork: https://github.com/Hero-Gamer/rbitcoin branch fix/628-txn-already-known
Upstream: reardencode/rbitcoin v31.1 pin 9be056a8a72b624dae9623b2f7bded92c2a21c91

## Prevent false positives - read these
- AGENTS.md: worktree branch, small commits, >=90% coverage, Red->Green->Refactor, no io_uring -> simple batch downgrade, on-disk changes need warn/schema bump
- TESTING.md: cargo test NEVER runs Core python tests. Core tests live in third_party/bitcoin submodule (v31.1). We do NOT copy 267 *.py files.
- docs/core-functional.md: scripts/core-functional/ holds inventory.toml, bitcoind shim, runner. run.sh only runs inventory `run` names with --v2transport. bitcoind shim: -datadir=DIR -> --datadir DIR/regtest, RBITCOIN_NODE env, stdio -> regtest/debug.log.
- docs/quality.md: 53 run, 214 skip.
- Inventory: name=*.py basename unique, status=run|skip, reason required on skip forbidden on run, never unknown.

## Issue #628
testmempoolaccept([raw of already-confirmed tx]) must return txn-already-known. Before fix returned txn-already-in-mempool/allowed.

Repro that passed:
BITCOIND=scripts/core-functional/bitcoind RBITCOIN_NODE=/tmp/rbtc-628/target/debug/rbitcoin-node python3 ./third_party/bitcoin/test/functional/test_628.py
-> result: {'reject-reason': 'txn-already-known'} PASS #628

## Check
1. Fix in rbitcoin-mempool / rbitcoin-query (confirm store lookup before mempool)
2. No edit to third_party/bitcoin, no .github/workflows push
3. cargo fmt, clippy -D warnings, cargo test -p rbitcoin-mempool green
4. Don't flip inventory.toml to rpc-dialect - this is a real fix
