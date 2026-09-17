# Agent notes

Documentation map (one owner per fact): [`docs/README.md`](docs/README.md).
This file is the harness-injected **hard-rule** contract. Design lives in
the owner docs; do not grow a second design book here.

## Plain technical language

Write **clear, concrete technical English** in code, comments, docs, commits,
and PR text. Do **not** inject moralizing, political framing, or performative
“sensitivity” language.

- **OK:** precise domain terms, plain failures (“reject”, “invalid”,
  “permanent blacklist”), Core-aligned vocabulary where we match Bitcoin Core.
- **Also OK when clearer:** `allowlist` / `denylist` for permitted or blocked
  names — for clarity, not as a ritual rename.
- **Not OK:** fashion rewrites, equity disclaimers, or softening
  consensus/security language.

If unsure whether a wording change is engineering clarity or cultural noise,
keep the existing technical term (especially if it matches Core or our logs).

## Comments are a smell

Do not restate *what* / *why* / *weird*. Prefer names, types, and structure.
Keep `//` only for an invariant, protocol rule, `SAFETY`, or library quirk.
Crate/public rustdoc (`//!` / `///`) that documents a surface is not this rule.

Full text and review checklist: [`CONTRIBUTING.md`](CONTRIBUTING.md) principle 7.

## Composition and immutability

Prefer composition (has-a) over inheritance (is-a); avoid tall trees. Prefer
immutable structures built once, then composed. If a hashmap needs extra
fields, wrap it on read rather than mutating members in place.

## Store

**No locks on the store hot path.** Roles, publish order, grow, pins:
[`docs/concurrency.md`](docs/concurrency.md). Heads: [`docs/heads.md`](docs/heads.md).
Class B insert geometry: [`SCHEMA.md`](SCHEMA.md) (Class B). Stage IO (the
only Allowed/Forbidden table): [`docs/invariants.md`](docs/invariants.md).

Do **not** reintroduce CreateResidency, OutFifo, ContigPark, archive sticky,
process pin FIFO, or map epochs. Do **not** add thread-local `test_take_*`
IO probes (session/table stats or file state). TxApply→dummy `Block`
conversion is `rbitcoin_query::testutil` only.

On-disk format change: [`SCHEMA.md`](SCHEMA.md) (soft migrate / `SCHEMA_VERSION`
bump / explicit refuse). Same commit as the format code. Do not surprise an
operator with a silent wipe.

io_uring machines: [`docs/io-modality.md`](docs/io-modality.md). Do **not**
flatten a purpose-built machine to batched `pread`/`pwrite` without asking.

Pin material is plan/batch only. IBD intake is body queue → lookup → load.
In-flight prune and leftover identity: [`docs/invariants.md`](docs/invariants.md).

Anything on lookup / load / scripts / write (or a sidecar the write thread
joins) gets a **named** `ibd: perf` timer **in the same commit**. Inventory:
`crates/rbitcoin-net/src/ibd/perf_log.rs`.

Process RAM vs page cache, body-queue soft assign, production evict APIs:
[`docs/ibd-memory.md`](docs/ibd-memory.md).

## Releases

**do a minor release** / **do a patch release** / **do a major release**
(or cut / tag a GitHub Release): follow [`docs/releases.md`](docs/releases.md).
That file is the only playbook (changelog cut, Highlights, tag, `vX.Y.x`,
`.99` bump). Do not copy it here.

## Ship via worktree + pull request

```text
one session worktree → topic branch per PR → many small commits → poll CI → green
```

A plan is **not complete** until that PR’s **required** checks are green.

**One git worktree per agent session**, not per PR. Reuse it so `target/dev`
stays warm. Topic branches still change per PR.

```bash
# once per session
git fetch origin
git worktree add /tmp/rbtc-<session> origin/master
cd /tmp/rbtc-<session>
git switch -c <area>/<short-name>
export CARGO_TARGET_DIR=/tmp/rbtc-<session>/target/dev
git config --worktree user.name 'rearden-grok[bot]'
git config --worktree user.email '317016512+rearden-grok[bot]@users.noreply.github.com'

# next PR in the same session (do not add another worktree)
git fetch origin
git switch -C <area>/<next-short> origin/master
```

| Rule | Detail |
|------|--------|
| Base | Current `origin/master` (or `main`) |
| Branch | Topic name — **never** commit the plan onto `master` |
| Worktree | **One** `/tmp/rbtc-<session>` for the session. Do **not** `git worktree add` per PR. |
| `CARGO_TARGET_DIR` | Inside the session worktree (`…/target/dev`) |
| Identity | Worktree-only `git config --worktree user.name` / `user.email` for bot commits |
| Remotes | Worktrees **share** `origin`. Fetch/pull is HTTPS; `pushurl` is SSH. Never `git remote set-url origin` (collapses the split). |

### After merge (keep the session worktree)

Once the PR is **merged** (not while open), delete the **topic branch** only:

```bash
git fetch origin --prune
git switch -C <area>/<next-short> origin/master   # or stay detached on origin/master
git branch -d <area>/<merged-short>
git push https://github.com/reardencode/rbitcoin.git --delete <area>/<merged-short>
```

Remove the worktree when the **session** ends, not after each PR:

```bash
git worktree remove /tmp/rbtc-<session>
git fetch origin --prune
```

Keep `master` / `main`, the primary checkout, and other sessions’ worktrees.
Do **not** delete a branch that still has an open PR. Do **not**
`git push --delete master`.

### Local tests (thin on purpose)

From `nix-shell` (CI pins **rustc 1.95.0**). Shell `CARGO_TARGET_DIR=target/dev`.
Suite, budgets, coverage: [`TESTING.md`](TESTING.md).

| When | Run |
|------|-----|
| **Each plan step / single-shot** | Targeted `cargo test -p <crate> …` (or slim scenario). `cargo fmt --all` if dirty. |
| **Compile inner loop** | `cargo check -p <crate> --lib` (or that same `--lib` test filter). **Not** `cargo check --tests` / multi-crate `--tests` after every edit. `--tests` once per green slice. |
| **Before push** | `cargo clippy --workspace --all-targets -- -D warnings` (same as CI `clippy`). Do not open a PR whose first clippy run is GitHub Actions. |
| **Core functional CI red** | Reproduce with `./scripts/core-functional/run.sh <failing.py>` against a cargo-built `rbitcoin-node` (`RBITCOIN_NODE=…/rbitcoin-node`). Rebuild the node after each product change. Iterate that script locally until it passes. Do **not** push and wait on the labeled job as the inner loop. Owner: [`docs/core-functional.md`](docs/core-functional.md). |
| **Not by default** | `cargo test --workspace`, `./scripts/coverage.sh`, `nix build .#rbitcoin-musl` |
| **Exception** | User asked for a local full suite, or you cannot push and must prove gates offline |

Do **not** wait out a host IBD or a coverage run in the agent VM. Coverage
stays a GitHub Actions gate. Clippy does not.

Do **not** delete a large type/module and chase `dead_code` / unresolved
across crates. Wrap the old API around the new one, switch one caller, delete
the leftover in **Refactor**. Checkpoint when `--lib` is green.
[`docs/how-we-plan.md`](docs/how-we-plan.md) (Keep the tree compiling).

### Push, PR, poll CI

Required jobs: **`fmt`**, **`deny`**, **`clippy`**, **`ast-grep`**, **`test`**,
**`windows`**, **`macos`**, **`coverage`**, **`nixos-module-eval`**. Structural scan is
`./scripts/ast-grep.sh` (rules live in `lint/ast-grep/`; do not copy them here).
`windows` / `macos` are native
store + `--smoke` (not operator zips). Operator binaries are GitHub Releases
(`release.yml`). Releases (minor / patch / major): [`docs/releases.md`](docs/releases.md).
Label **`core-functional`** when the PR touches the Core functional harness
(and on every **ship** version-bump PR). Do **not** label ordinary net or
RPC PRs; the job is too slow for the default gate. Default `cargo test` is
the pin; owner rules: [`TESTING.md`](TESTING.md) (Default CI is the pin).

`origin` fetch/pull is HTTPS; `pushurl` is SSH (operator). This VM has **no**
GitHub App SSH key. The App token from `~/.config/rbitcoin-grok/gh-login.sh`
(~1h) is HTTPS-only.

`gh pr create` / `gh pr checks` / `gh issue comment` / `gh run rerun` talk
to the API. `git fetch origin` works here. Bot **push** must use an
**explicit HTTPS URL**. Do **not** `git remote set-url origin`. Do **not**
`git push origin` as the bot.

Installation write: `contents`, `pull_requests`, `issues`, `workflows`,
`actions`. Push `.github/workflows/*` with the rest of the topic branch.
Do not strip a workflow diff to sneak a push. `gh run rerun` is in-band.
Read: `checks`, `security_events`, `secret_scanning_alerts`, and the other
listed reads. CodeQL **dismiss** stays operator (`security_events` is read).

```bash
~/.config/rbitcoin-grok/gh-login.sh
git fetch origin
git push https://github.com/reardencode/rbitcoin.git HEAD:<area>/<short-name>
gh pr create --repo reardencode/rbitcoin --head <area>/<short-name> --title "…" --body "…"
gh pr view --repo reardencode/rbitcoin --json mergeable,mergeStateStatus
./scripts/pr-checks-watch.sh --repo reardencode/rbitcoin --pr <n> \
  --interest windows
```

No `-u` on push (that would retarget the branch remote away from `origin`).

**Fail-fast poll.** Do **not** `gh pr checks --watch` as the only waiter. That
blocks until *every* check finishes (slow `coverage`, CodeQL, `Analyze (rust)`)
even after a job this change was meant to exercise (`windows`, `macos`,
`clippy`, `test`, …) is already red. `./scripts/pr-checks-watch.sh` exits **1
as soon as any required job fails**, and also as soon as a `--interest` job
fails (pass the job(s) this change exercises). On exit 1: **stop polling**,
fetch that job’s log **in this session**, and start the fix. Do not wait for
coverage, macos, test, or CodeQL. The script prints the job URL.

```bash
# Watcher printed "start the fix now" + job URL (run may still be in progress):
gh api --allow-escape-sequences repos/reardencode/rbitcoin/actions/jobs/<job-id>/logs
# after the run finishes:
gh run view <run-id> --repo reardencode/rbitcoin --job <job-id> --log-failed
```

**Unmergeable ⇒ tests do not run.** If `mergeable` is `CONFLICTING` or
`mergeStateStatus` is `DIRTY` (or `BEHIND` when the branch is not on current
`origin/master`), required **test** CI does not start. `./scripts/pr-checks-watch.sh`
will sit on pending `test` — that is not a flake. Rebase onto
`origin/master`, lease-force the topic branch, then poll. Do not `gh run rerun`
to wake jobs that never queued.

```bash
git fetch origin
git rebase origin/master
git push --force-with-lease https://github.com/reardencode/rbitcoin.git HEAD:<area>/<short-name>
```

| Rule | Detail |
|------|--------|
| **One PR per plan** | Push more commits to the same branch. |
| **Mergeable first** | Conflicted / behind PRs skip test CI. Rebase, then poll. |
| **Poll until green** | Fail-fast: `--interest` the job this change exercises. On that (or any required) fail, fetch the log and start the fix — do not wait out coverage/CodeQL on a red `windows`. Script: `./scripts/pr-checks-watch.sh`. |
| **Done** | Required checks green **and** the PR is up for review. Do not merge unless asked. |
| **No post-green PR-cite** | After required checks are green, do **not** push a docs-only follow-up whose only change is inserting this PR's number into CHANGELOG / quality.md / similar. That wastes a full CI run. Cite in the **PR body**. Owner docs can omit the GitHub number, or pick it up later in a docs change that was already needed. |
| **Do not** | Force-push `master`, merge a red PR, collapse `origin` to a single URL, skip polling because “tests passed locally,” or invent **empty commits** to poke Actions. |
| **Workflow YAML** | Push with the topic branch (`workflows:write`). |
| **CodeQL in tests** | Alert that only fires in `#[cfg(test)]` / test modules: **stop**. Do **not** rename tests or shuffle literals to silence it. Ask the operator to **dismiss** (`security_events` is read). Production / library CodeQL is a real finding — fix it. |

#### Retrigger CI (no empty commits)

When required checks are green locally and CI only needs a re-run (flake,
stale run):

| OK | Not OK |
|----|--------|
| `gh run rerun <id> [--failed]` | Empty commit whose only purpose is to wake Actions |
| GitHub Actions UI **Re-run failed jobs** / **Re-run all jobs** | Noise commits (“ci: bump”, “trigger”) with no product/test change; docs-only follow-up whose only change is this PR's `#N` after checks are already green |
| Amend the tip commit (or rebase) and **force-push the topic branch** with `--force-with-lease` over HTTPS | Force-push `master` / `main` |

```bash
gh run rerun <run-id> --failed

# If the API rejects: amend tip (no empty commit) and lease-force the topic branch only:
git commit --amend --no-edit   # or fold a real fix into the tip
git push --force-with-lease https://github.com/reardencode/rbitcoin.git HEAD:<area>/<short-name>
```

Coverage (LCOV `LH`/`LF` never below last green `master`) is a required CI
job — see [`TESTING.md`](TESTING.md). If CI `coverage` fails, add a pin and
push.

Plans: [`docs/how-we-plan.md`](docs/how-we-plan.md). Each step names
**Contract, Red, Green, Refactor, Verify**. Many small vertical slices.

## Commit hygiene

This tree is **public**.

| Rule | Detail |
|------|--------|
| **One logical change** | One concern per commit. |
| **Small** | Sequence of small commits; checkpoint before risky follow-ons. |
| **Clear message** | Subject + body: **what** and **why**. No chat context assumed. |
| **Not** | “WIP”, “misc”, drive-by renames mixed with behavior. |

Green-then-refactor is fine as **two** commits when each stands alone.

1. Pass targeted tests for what you touched.
2. Commit. A plan is **many commits, one PR**.
3. `cargo clippy --workspace --all-targets -- -D warnings`, then push the
   topic branch (same session worktree) and open or update the plan PR. Poll
   to green.

Operator musl/release binaries are [`docs/releases.md`](docs/releases.md) /
[`docs/reproducible-builds.md`](docs/reproducible-builds.md) — not a plan-PR
step. Do **not** `nix build .#rbitcoin-musl` on a feature branch.

## Test-driven development

**No production code change without a test that fails first.** Pure
docs/comments/formatting need no tests. Do **not** open a mainnet datadir in
the agent VM. Perf A/B is operator-host only.

| Phase | Goal | Rules |
|-------|------|--------|
| **Red** | Encode the contract | Failing test only. No production edit yet. |
| **Green** | Make it pass | **Smallest surgical** change. One-offs OK *temporarily*. Keep `--lib` compiling: wrap old APIs; do not delete them until callers have moved. |
| **Refactor** | Remove the one-off | Still green: fold into the real shape; delete dual paths. |

Planning anatomy, INVEST, step template: [`docs/how-we-plan.md`](docs/how-we-plan.md).
Fixture size, one-entry-per-path, coverage bar, default-CI vs nightly Core:
[`TESTING.md`](TESTING.md). Extend a catalog journey; do not add a twin or
treat Core functional as the PR pin.

The test must assert the **exact** contract, drive the **shipped** function,
fail with the **same class of error**, and use tiny `/tmp` fixtures.

## Lean-code rules

One production implementation at the **lowest crate** that owns the concept.
Missing promised fact → `StoreError::Corrupt("invariant: …")` — no silent
fallback. Spentness / same-block / pin identity:
[`docs/invariants.md`](docs/invariants.md). Tests assert shipped behavior, not
repo text ([`CONTRIBUTING.md`](CONTRIBUTING.md) principle 8);
[`TESTING.md`](TESTING.md) owns budgets, no `*_for_test` backdoors, and no
production-scale default fixtures. Do not waste RAM or CPU; a spend of one
to save the other is a named trade ([`CONTRIBUTING.md`](CONTRIBUTING.md)
principle 9). Crate `pub` is the cross-crate graph only — unused `pub` is
forbidden; `#[cfg(test)]` on production items is a smell; fuzz-only
exports are the same smell ([`CONTRIBUTING.md`](CONTRIBUTING.md)
principle 11).

Do not leave dead code. Do not silence dead-code warnings — delete the
code. Do not wrap unused production APIs in `#[cfg(test)]` to keep them.
