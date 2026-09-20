# cargo-mutants snapshot

How to run and what CI does: [`../../TESTING.md`](../../TESTING.md)
(Mutation testing). This directory is a **generated** missed/timeout list
from a host run, not the quality roadmap
([`../quality.md`](../quality.md)).

Generated: 2026-09-20 16:14 ET. Host: M4 mini. Consensus 2026-09-19 night
→ 2026-09-20 morning (9h, 2518 tested). Primitives 2026-09-20 ~15:00 ET
(16m, 292 tested).

| Crate | Tested | Missed | Timeout | Details |
| :--- | ---: | ---: | ---: | :--- |
| `rbitcoin-primitives` | 292 | 4 | 8 | [./rbitcoin-primitives.md](./rbitcoin-primitives.md) |
| `rbitcoin-consensus` | 2518 | 304 | 38 | [./rbitcoin-consensus.md](./rbitcoin-consensus.md) |

### Top files — consensus

- `block/mod.rs` 65 · `script/interpreter.rs` 57 · `silent_payments.rs` 50 · `confirm_run/write.rs` 23 · `confirm_run/pin.rs` 22
