# rbitcoin-query

Confirm and query layer over `rbitcoin-store`. Consensus, net, RPC, Electrum,
and Esplora call this crate. They do not talk to store tables directly when
a query API already owns the fact.

## Read first

| Change | Read |
|--------|------|
| Stage IO, pin identity, leftover | [`docs/invariants.md`](../../docs/invariants.md) |
| Body queue, roles, pins | [`docs/concurrency.md`](../../docs/concurrency.md) |
| RAM caps and evict | [`docs/ibd-memory.md`](../../docs/ibd-memory.md) |
| Known one-off quirks | [`docs/errata.md`](../../docs/errata.md) |

## Where

- Stamp and parents: `src/stamp.rs`, `src/batch_parents.rs`, `src/in_flight.rs`
- Load: `src/confirm_load.rs`, `src/confirm_parent_cache.rs`
- Archive: `src/archive.rs`
- Fixtures: `src/testutil.rs`

## Rules here

- Pins are plan/batch only. No process-resident FIFO, LRU, or sticky pin cache.
- Test-only adapters stay in `testutil`. Do not grow production `Query` around fixture shapes.
- A missed fact the pipeline promised is `StoreError::Corrupt("invariant: …")`, not a silent colder walk.

## Verify

`cargo test -p rbitcoin-query --lib`
