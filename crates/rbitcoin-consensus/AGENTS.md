# rbitcoin-consensus

Header, block, and script validation. Depends on primitives, query, and store.
Peer and RPC script checks use the detached worker (`verify_tx_scripts_detached`).
Do not run the interpreter on an I/O thread.

## Read first

| Change | Read |
|--------|------|
| Rules we own vs Core corpora | [`docs/consensus-tests.md`](../../docs/consensus-tests.md) |
| rust-bitcoin gaps | [`docs/rust-bitcoin-limitations.md`](../../docs/rust-bitcoin-limitations.md) |
| Do not split an opcode match for line count | [`docs/code-shape.md`](../../docs/code-shape.md) |

## Where

- Scripts: `src/script/` (`interpreter.rs` is the opcode match)
- Confirm: `src/confirm_run.rs`
- Headers and blocks: `src/header.rs`, `src/block.rs`
- Worker pool: `src/script_pool.rs`

## Rules here

- One production implementation. No silent consensus fallback.
- Do not split a Core-faithful opcode `match` to chase a line count.

## Verify

`cargo test -p rbitcoin-consensus --lib`
