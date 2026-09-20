# Agent notes

Documentation map (one owner per fact): [`docs/README.md`](docs/README.md).
Before exploring, read [`docs/ORIENT.md`](docs/ORIENT.md).
This file is the harness-injected hard-rule contract. Design lives in the
owner docs; do not grow a second design book here.

Process playbooks are Agent Skills under [`.agents/skills/`](.agents/skills/).
Read the matching `SKILL.md` when the task needs it. Do not paste bash,
tables, or command cribs from those files here. Short hard-rule lines that
wreck a session if missed stay in this file; the owner playbook stays in
the skill.

If you are acting as `rearden-grok[bot]`, also read
[`rearden-vm-HOST.md`](rearden-vm-HOST.md). If you are not that identity,
ignore that file.

## Language, comments, composition

Write clear, concrete technical English. Keep Core-aligned terms where we
match Bitcoin Core. Do not inject moralizing or political framing, or soften
consensus and security language. If a rename is not clearer engineering, keep
the existing term.

Comments that restate what, why, or weird are a smell. Prefer names, types,
and structure. Keep `//` only for an invariant, protocol rule, `SAFETY`, or
library quirk. Crate and public rustdoc (`//!` / `///`) is not this rule.
Full text: [`CONTRIBUTING.md`](CONTRIBUTING.md) principle 7.

Prefer composition (has-a) over inheritance; avoid tall trees. Build
immutable structures once, then compose them. If a map needs extra fields,
wrap it on read rather than mutating members in place.
Control flow: principle 10 and [`docs/code-shape.md`](docs/code-shape.md).

## Store and IBD

**No locks on the store hot path.** Roles, publish order, grow, pins:
[`docs/concurrency.md`](docs/concurrency.md). Heads: [`docs/heads.md`](docs/heads.md).
Class B insert geometry: [`SCHEMA.md`](SCHEMA.md). Stage IO (the only
Allowed/Forbidden table): [`docs/invariants.md`](docs/invariants.md).

- No large process-resident body, pin, or archive caches with FIFO, LRU, or
  sticky residency. Pins are plan/batch only. IBD intake is body queue →
  lookup → load. [`docs/ibd-memory.md`](docs/ibd-memory.md).
- Grow capacity with fallocate / `set_len` and a published high-water mark.
  Do not introduce remap-epoch schemes. [`docs/concurrency.md`](docs/concurrency.md).
- Do not replace a purpose-built IO machine with generic batched `pread` /
  `pwrite` without asking. [`docs/io-modality.md`](docs/io-modality.md).
- Missing promised fact → `StoreError::Corrupt("invariant: …")`. No silent
  fallback. On-disk change: soft migrate, `SCHEMA_VERSION` bump, or explicit
  refuse, in the same commit as the format code. Never a silent wipe.
  [`SCHEMA.md`](SCHEMA.md).
- Test-only adapters stay in `*_testutil`. Do not grow production APIs around
  fixture shapes.
- Anything on lookup, load, scripts, or write (or a sidecar the write thread
  joins) gets a named `ibd: perf` timer in the same commit. Inventory:
  `crates/rbitcoin-net/src/ibd/perf_log.rs`.

## Change discipline

No production code change without a test that fails first. Docs, comments,
and formatting need no tests. Do not open a mainnet datadir in the agent VM.
Perf A/B is operator-host only.

One plan step is one Red → Green → Refactor turn, committed before the next
step. Keep `--lib` compiling (wrap the old API, switch one caller). Inner
loop is `cargo test -p <crate> --lib` / `cargo check -p <crate> --lib`.
Do not `cargo check --tests` after every edit. Owner:
[`docs/how-we-plan.md`](docs/how-we-plan.md). Commands:
[`.agents/skills/ship-pr/SKILL.md`](.agents/skills/ship-pr/SKILL.md).

**Agent RAM:** never load `cargo test`, clippy, deny, or rustc stdout into
the session. Redirect stdout and stderr to a file under `/tmp`; read the
exit code, failure names (`test … FAILED`, lint ids, first rustc error),
and at most ~80 lines of tail. `--quiet` is enough to confirm green.
Owner: [`docs/how-we-plan.md`](docs/how-we-plan.md) (Agent RAM).

**Agent disk:** `rearden-grok[bot]` follows [`rearden-vm-HOST.md`](rearden-vm-HOST.md)
(one worktree, shared silo, no cargo in the Cursor checkout). Other
identities: ignore that file; use `$PWD/target/dev`.

One production implementation at the lowest crate that owns the concept.
Extract is a move: [`docs/code-shape.md`](docs/code-shape.md). Core-facing
RPC / P2P / Electrum / Esplora: [`COMPAT.md`](COMPAT.md).

Tests assert shipped behavior, not repo text
([`CONTRIBUTING.md`](CONTRIBUTING.md) principle 8). Budgets, no `*_for_test`
backdoors, no production-scale default fixtures: [`TESTING.md`](TESTING.md).
Tests use session or table instance stats or on-disk state, not thread-local
hot-path IO probes.

Crate `pub` is the cross-crate graph only. Unused `pub` is forbidden.
`#[cfg(test)]` on production items is a smell, including fuzz-only exports.
Do not leave dead code or silence `dead_code`. A RAM or CPU trade is named
([`CONTRIBUTING.md`](CONTRIBUTING.md) principle 9).

One logical change per commit. The message says what and why. Not WIP, misc,
or a drive-by rename mixed with behavior.

Before the first edit under `crates/<name>/`, read `crates/<name>/AGENTS.md`
when it exists.

## Ship, release, Core functional

One session worktree, one topic branch per PR, required checks green before
the plan is done. Do not merge unless asked. Never commit the plan onto
`master`.

These wreck a session even when the skill was not opened:

- Fail-fast poll: `./scripts/pr-checks-watch.sh`. Do not `gh pr checks --watch`
  (it waits out coverage and CodeQL after a job this change already failed).
- Conflicted or behind PRs skip test CI. Rebase onto `origin/master`, then
  `--force-with-lease` the topic branch. Do not `gh run rerun` jobs that
  never queued.
- Run clippy `-D warnings` before push. Do not wait out
  `./scripts/coverage.sh` or a host IBD. No empty commits to poke Actions
  (`gh run rerun` instead).
- `rearden-grok[bot]` only (ignore [`rearden-vm-HOST.md`](rearden-vm-HOST.md)
  otherwise): never `git remote set-url origin`; HTTPS `HEAD:<branch>` push,
  not `git push origin`, no `-u`; one `/tmp/rbtc-<session>` worktree;
  `CARGO_TARGET_DIR=/tmp/rbtc-target/dev` before `nix-shell`; do not cargo
  in the Cursor checkout. Owner: [`rearden-vm-HOST.md`](rearden-vm-HOST.md).

Playbooks:

- Opening, updating, pushing, or polling a PR:
  [`.agents/skills/ship-pr/SKILL.md`](.agents/skills/ship-pr/SKILL.md).
- A minor, patch, or major release:
  [`.agents/skills/release/SKILL.md`](.agents/skills/release/SKILL.md).
  Owner playbook: [`docs/releases.md`](docs/releases.md).
- Core functional harness:
  [`.agents/skills/core-functional/SKILL.md`](.agents/skills/core-functional/SKILL.md).
