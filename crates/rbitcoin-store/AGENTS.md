# rbitcoin-store

Map-free relational archive: Class A append bodies, Class B mutable hash
heads, Class C tip-mutable confirmed state. Depends on `rbitcoin-primitives`
and `rbitcoin-log`. `rbitcoin-query` and everything above it call this crate.

## Read first

| Change | Read |
|--------|------|
| Stage IO, leftover union, no silent fallback | [`docs/invariants.md`](../../docs/invariants.md) |
| Roles, publish order, locks, grow | [`docs/concurrency.md`](../../docs/concurrency.md) |
| Bytes, Class B geometry, schema bump | [`SCHEMA.md`](../../SCHEMA.md) |
| Which head file | [`docs/heads.md`](../../docs/heads.md) |
| fd vs io_uring | [`docs/io-modality.md`](../../docs/io-modality.md) |
| Tip-as-commit, kill-9 | [`docs/crash-recovery.md`](../../docs/crash-recovery.md) |

## Where

- Open and chain: `src/chain.rs`, `src/file.rs`
- IO: `src/io_backend.rs`, `src/io_session_pool.rs`
- Heads: `src/hashhead.rs`, `src/header_table.rs`, `src/scripthash.rs`
- Fixtures: `src/testutil.rs` (`TempDir`, `tiny_store`)

## Rules here

- No locks on the hot path.
- No large process-resident body or pin caches (FIFO, LRU, or sticky residency).
- Grow files with fallocate / `set_len` and a published high-water mark. No remap-epoch schemes.
- Do not replace a purpose-built IO machine with batched `pread` / `pwrite` without asking.
- Schema change in the same commit: soft migrate, bump, or refuse. Never a silent wipe.

## Verify

`cargo test -p rbitcoin-store --lib`
