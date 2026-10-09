# rbitcoin-consensus

Header, block, and script validation. Depends on primitives, query, and store.
Peer and RPC script checks use the detached worker. The thread rule is
[`docs/concurrency.md`](../../docs/concurrency.md).

## Read first

Open the row that matches the change. Leave the other owners closed.

| Change | Read |
|--------|------|
| Rules we own vs Core corpora | [`docs/consensus-tests.md`](../../docs/consensus-tests.md) |
| rust-bitcoin gaps | [`docs/rust-bitcoin-limitations.md`](../../docs/rust-bitcoin-limitations.md) |
| Do not split an opcode match for line count | [`docs/code-shape.md`](../../docs/code-shape.md) |

## Where

- Scripts: `src/script/` (`interpreter.rs` is the opcode match)
- Confirm: `src/confirm_run/`
- Headers and blocks: `src/header.rs`, `src/block/`
- Worker pool: `src/script_pool.rs`

## Verify

`cargo test -p rbitcoin-consensus --lib`
