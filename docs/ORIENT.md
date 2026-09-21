# Agent orientation

Open this when the task area is unclear.
Facts stay in the owner files in [`README.md`](./README.md). This page only routes.
Open the one row that matches the change. Leave the other owners closed.

## Crate graph

`rbitcoin-primitives` and `rbitcoin-log` sit under the rest.
`rbitcoin-store` is the map-free relational archive (Class A append, Class B hash heads, Class C tip-mutable).
`rbitcoin-query` is the confirm and query layer over that store.
`rbitcoin-consensus` validates headers, blocks, and scripts.
`rbitcoin-mempool` is the live transaction graph.
`rbitcoin-net` is P2P and IBD (query, store, consensus, mempool).
`rbitcoin-rpc`, `rbitcoin-electrum`, and `rbitcoin-esplora` serve query and net.
`rbitcoin-node` composes those crates into one process.
`rbitcoin-cli` is the RPC client (primitives only).
`rbitcoin-test` and `rbitcoin-bench` are harnesses, not the product graph.

Before the first edit in a crate, read `crates/<name>/AGENTS.md` when that file exists.

## Read first

| Task | Read |
|------|------|
| Confirm stage IO, leftover union, pin identity, silent fallback | [`invariants.md`](./invariants.md) |
| Writer roles, publish order, body queue, pins | [`concurrency.md`](./concurrency.md) |
| On-disk bytes, schema bump / migrate / refuse | [`../SCHEMA.md`](../SCHEMA.md) |
| Process RAM, body-queue caps, production evict | [`ibd-memory.md`](./ibd-memory.md) |
| io_uring vs fd; do not flatten a purpose-built machine | [`io-modality.md`](./io-modality.md) |
| Which head file (tx / header / scripthash) | [`heads.md`](./heads.md) |
| Crash, tip-as-commit, kill-9 | [`crash-recovery.md`](./crash-recovery.md) |
| Tests, budgets, coverage, fixtures | [`../TESTING.md`](../TESTING.md) |
| A missed cargo-mutants case | [`../TESTING.md`](../TESTING.md) (Mutation testing). Open [`mutants/`](./mutants/) only for that miss |
| Multi-step plan (Red → Green → Refactor) | [`how-we-plan.md`](./how-we-plan.md#agent-contract) (stop before Rationale) |
| cargo / clippy / deny / rustc logs (do not load into the session) | [`how-we-plan.md`](./how-we-plan.md) (Agent RAM) |
| `rearden-grok[bot]` operator VM (ignore unless that identity) | [`../rearden-vm-HOST.md`](../rearden-vm-HOST.md) |
| Intentional Core / Electrum / Esplora differences | [`../COMPAT.md`](../COMPAT.md) |
| JSON-RPC surface | [`rpc.md`](./rpc.md) |
| Open, push, or poll a PR | [`../.agents/skills/ship-pr/SKILL.md`](../.agents/skills/ship-pr/SKILL.md) (one worktree; fail-fast `pr-checks-watch.sh`) |
| Minor, patch, or major release | [`../.agents/skills/release/SKILL.md`](../.agents/skills/release/SKILL.md) |
| Core functional harness | [`../.agents/skills/core-functional/SKILL.md`](../.agents/skills/core-functional/SKILL.md) |
| Overlay functional harness (private Tor / i2pd / cjdns) | [`../.agents/skills/overlay-functional/SKILL.md`](../.agents/skills/overlay-functional/SKILL.md) |

## Ask first

Hard stops in [`../AGENTS.md`](../AGENTS.md) are proceed-or-stop rules.
`rearden-grok[bot]` also follows [`../rearden-vm-HOST.md`](../rearden-vm-HOST.md);
other identities ignore that file.

Ask before any of these, including when the change looks locally justified:

- Replace a purpose-built IO machine with batched `pread` / `pwrite`.
- Bump or wipe the schema, or add an `RBITCOIN_*` knob.
- Widen who is trusted beyond [`../SECURITY.md`](../SECURITY.md).
- Merge, force-push `master`, or `cargo clean` the shared cargo silo.
- Start an item in [`quality.md`](./quality.md), [`peer-clients.md`](./peer-clients.md),
  [`personal-node-plans/`](./personal-node-plans/), or [`road-to-1.0.md`](./road-to-1.0.md)
  that the user did not name.
- Open a mainnet datadir, or treat an agent-VM run as a perf result.
- Label an ordinary net or RPC pull request `core-functional`.
