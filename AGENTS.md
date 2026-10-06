# Agent notes

Documentation map (one owner per fact): [`docs/README.md`](docs/README.md).
When the task area is unclear, [`docs/ORIENT.md`](docs/ORIENT.md) routes.
Open the one row that matches the change. This file is the harness-injected
hard-rule contract. It points. Design and procedures stay in the owner doc
or the skill. Do not paste them here.

Process playbooks: [`.agents/skills/`](.agents/skills/). Read the matching
`SKILL.md` when the task needs it.

If you are acting as `rearden-grok[bot]`, also read
[`rearden-vm-HOST.md`](rearden-vm-HOST.md). If you are not that identity,
ignore that file.

## Where to read

| You are about to | Open |
|------------------|------|
| Edit Rust | [`CONTRIBUTING.md`](CONTRIBUTING.md) principles 7–11 and [`docs/code-shape.md`](docs/code-shape.md) |
| Edit `crates/<name>/` | `crates/<name>/AGENTS.md` when it exists, then its one read-first row |
| Touch store, IBD, schema, or IO | The matching row in [`docs/ORIENT.md`](docs/ORIENT.md) |
| Plan more than one step | [`docs/how-we-plan.md`](docs/how-we-plan.md#agent-contract) (stop before Rationale) |
| Open, push, or poll a PR | [`.agents/skills/ship-pr/SKILL.md`](.agents/skills/ship-pr/SKILL.md) |
| Cut a release | [`.agents/skills/release/SKILL.md`](.agents/skills/release/SKILL.md) and [`docs/releases.md`](docs/releases.md) |
| Touch a labeled harness | The matching skill under [`.agents/skills/`](.agents/skills/) |

## Hard stops

These wreck a session if you skip them. The owner has the procedure.

- No production change without a test that fails first. Docs, comments, and formatting need no tests. Do not open a mainnet datadir here. Perf A/B is operator-host only. Cycle: [`docs/how-we-plan.md`](docs/how-we-plan.md#agent-contract).
- Do not load `cargo test`, clippy, deny, or rustc stdout into the session. Redirect to `/tmp`; read the exit code, failure names, and at most ~80 lines of tail. [`docs/how-we-plan.md`](docs/how-we-plan.md) (Agent RAM).
- Do not run `cargo mutants`. Hand-edit the missed operator and run the journey. [`TESTING.md`](TESTING.md). `rearden-grok[bot]`: [`rearden-vm-HOST.md`](rearden-vm-HOST.md).
- Do not merge unless asked. Never commit the plan onto `master`.
- Poll with `./scripts/pr-checks-watch.sh`. Do not `gh pr checks --watch`. A conflicted or behind PR skips test CI: rebase onto `origin/master`, then `--force-with-lease` the topic branch. Do not `gh run rerun` jobs that never queued. [`.agents/skills/ship-pr/SKILL.md`](.agents/skills/ship-pr/SKILL.md).
- Unreleased notes are a new file under `changelog.d/`. Do not edit `CHANGELOG.md` on a feature branch. [`docs/releases.md`](docs/releases.md).
- `rearden-grok[bot]` only: never `git remote set-url origin`; push `HEAD:<branch>` over HTTPS, not `git push origin`. One `/tmp/rbtc-<session>` worktree. Leave `CARGO_TARGET_DIR` unset. Do not cargo in the Cursor checkout. [`rearden-vm-HOST.md`](rearden-vm-HOST.md).

Ask before the changes listed in [`docs/ORIENT.md`](docs/ORIENT.md) (Ask first).
