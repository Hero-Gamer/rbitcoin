---
name: ship-pr
description: >-
  Open, update, push, or poll a pull request. Use for session worktrees,
  topic branches, HTTPS bot push, fail-fast CI, rebase when unmergeable,
  and retriggering a flake without an empty commit.
---

# Ship a pull request

One session worktree, one topic branch per plan, many small commits (each
already passed local CI except coverage), required checks green. A plan is
not done until those checks are green. Do not merge unless asked.

Owner facts for tests and coverage stay in [`TESTING.md`](../../../TESTING.md).
Do not copy them here.

## Session worktree

Reuse one worktree per session. Do not add a worktree per PR.

`rearden-grok[bot]` on this operator VM: silo, editor tree, ENOSPC, identity,
and HTTPS push are [`rearden-vm-HOST.md`](../../../rearden-vm-HOST.md). If
you are not that identity, ignore that file; use `$PWD/target/dev`.

```bash
# once per session
git fetch origin
git worktree add /tmp/rbtc-<session> origin/master
cd /tmp/rbtc-<session>
git switch -c <area>/<short-name>
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target/dev}"

# next PR in the same session (do not add another worktree)
git fetch origin
git switch -C <area>/<next-short> origin/master
```

| Rule | Detail |
|------|--------|
| Base | Current `origin/master` (or `main`) |
| Branch | Topic name. Never commit the plan onto `master` |
| Worktree | One `/tmp/rbtc-<session>` for the session |
| `CARGO_TARGET_DIR` | `$PWD/target/dev` unless already set. `rearden-grok[bot]`: [`rearden-vm-HOST.md`](../../../rearden-vm-HOST.md) |
| Remotes | Worktrees share `origin`. Never `git remote set-url origin` |

### After merge

Delete the topic branch only. Keep the session worktree until the session ends.
Do not delete a branch that still has an open PR. Do not `git push --delete master`.

```bash
git fetch origin --prune
git switch -C <area>/<next-short> origin/master
git branch -d <area>/<merged-short>
# rearden-grok[bot]: delete the topic branch over HTTPS (rearden-vm-HOST.md)
```

```bash
# session end
git worktree remove /tmp/rbtc-<session>
git fetch origin --prune
```

`rearden-grok[bot]` session-end extras (test dirs, shared silo):
[`rearden-vm-HOST.md`](../../../rearden-vm-HOST.md). Other identities:
ignore that file.

## Local tests

From `nix-shell` (CI pins rustc 1.95.0). `CARGO_TARGET_DIR` as in Session
worktree. Same **commands** as the required jobs, not the GitHub Actions
`env:`
(leave `CARGO_INCREMENTAL` unset; that is [`ci.yml`](../../../.github/workflows/ci.yml) only).
When to run which command:
[`docs/how-we-plan.md`](../../../docs/how-we-plan.md) (The cycle).
Suite and budgets: [`TESTING.md`](../../../TESTING.md).

| When | Run |
|------|-----|
| Inner loop (Red / Green) | Targeted `cargo test -p <crate> … -- --quiet`. `cargo check -p <crate> --lib`. Not `--tests` after every edit |
| After Green | `cargo test --workspace --quiet` |
| After Refactor, before commit | Local CI except coverage (below) |
| Core functional CI red | [core-functional skill](../core-functional/SKILL.md) |
| Never local | `./scripts/coverage.sh`, `nix build .#rbitcoin-musl`, host IBD |

Redirect every command above (stdout and stderr to `/tmp`, print `EXIT`,
search then tail on failure). Do not `tee` or `| tail` as the only sink,
and do not load the log wholesale. Recipe:
[`docs/how-we-plan.md`](../../../docs/how-we-plan.md) (Agent RAM). Do not
copy that snippet here.

Local CI except coverage:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo deny check
./scripts/ast-grep.sh
cargo test --workspace --quiet
./scripts/ci-os-smoke.sh
```

If the slice changed `flake.nix`, `nix/`, or the NixOS module, also
`nix build .#checks.x86_64-linux.nixos-module-eval --no-link`. Label the PR
`nixos-module-runtime` when systemd deps, users/groups, firewall, or the VM
start argv changed — that runs `nixos-module-runtime` (qemu). Do not wait
out the VM test locally.

Coverage and native `windows` / `macos` stay GitHub Actions;
`ci-os-smoke.sh` is the local stand-in. Do not wait out a host IBD or a
coverage run in the agent VM.

## Push and poll

Required jobs: `fmt`, `deny`, `clippy`, `ast-grep`, `test`, `windows`,
`macos`, `coverage`, `nixos-module-eval`. Structural scan is
`./scripts/ast-grep.sh`. `windows` / `macos` are native store plus `--smoke`,
not operator zips.

Label `core-functional` when the PR touches that harness, and on every ship
version-bump PR. Do not label ordinary net or RPC PRs.
Label `nixos-module-runtime` when the NixOS module VM test should run (not
eval). Poll with `--interest nixos-module-runtime`. It is not a required
check.

`rearden-grok[bot]` fetch/push (`gh-login.sh`, HTTPS `HEAD:<branch>`, never
`git push origin`, no `-u`): [`rearden-vm-HOST.md`](../../../rearden-vm-HOST.md).
If you are not that identity, ignore that file.

```bash
gh pr create --repo reardencode/rbitcoin --head <area>/<short-name> --title "…" --body "…"
gh pr view --repo reardencode/rbitcoin --json mergeable,mergeStateStatus
./scripts/pr-checks-watch.sh --repo reardencode/rbitcoin --pr <n> \
  --interest windows
```

Installation write: `contents`, `pull_requests`, `issues`, `workflows`,
`actions`. Push `.github/workflows/*` with the topic branch. Do not strip a
workflow diff to sneak a push. `gh run rerun` is in-band. CodeQL dismiss stays
operator (`security_events` is read). An alert that only fires in `#[cfg(test)]`
or a test module: stop. Do not rename tests or shuffle literals. Ask the
operator to dismiss. Production CodeQL is a real finding — fix it.

**Fail-fast poll.** Do not use `gh pr checks --watch` as the only waiter. It
blocks until every check finishes, including slow `coverage` and CodeQL, even
after a job this change exercises is already red. `./scripts/pr-checks-watch.sh`
exits 1 as soon as any required job fails, and as soon as a `--interest` job
fails. On exit 1: stop polling, fetch that job’s log, and start the fix.

```bash
gh api --allow-escape-sequences repos/reardencode/rbitcoin/actions/jobs/<job-id>/logs
gh run view <run-id> --repo reardencode/rbitcoin --job <job-id> --log-failed
```

**Unmergeable means tests do not run.** If `mergeable` is `CONFLICTING` or
`mergeStateStatus` is `DIRTY` (or `BEHIND` when the branch is not on current
`origin/master`), required `test` CI does not start. Rebase, lease-force the
topic branch, then poll. Do not `gh run rerun` to wake jobs that never queued.

```bash
git fetch origin
git rebase origin/master
# rearden-grok[bot]: lease-force over HTTPS (rearden-vm-HOST.md)
```

| Rule | Detail |
|------|--------|
| One PR per plan | Push more commits to the same branch |
| Mergeable first | Conflicted or behind PRs skip test CI. Rebase, then poll |
| Poll until green | `--interest` the job this change exercises. On fail, fetch the log and fix |
| Done | Required checks green and the PR is up for review. Do not merge unless asked |
| No post-green PR-cite | Do not push a docs-only commit whose only change is this PR's number. Cite it in the PR body |
| Do not | Force-push `master`, merge a red PR, collapse `origin` to one URL, skip polling, or invent empty commits to poke Actions |

### Retrigger CI

When required checks are green locally and CI only needs a re-run:

| OK | Not OK |
|----|--------|
| `gh run rerun <id> [--failed]` | Empty commit whose only purpose is to wake Actions |
| GitHub Actions re-run failed or all jobs | Noise commits (`ci: bump`, `trigger`) |
| Amend the tip and `--force-with-lease` the topic branch over HTTPS | Force-push `master` / `main` |

```bash
gh run rerun <run-id> --failed

# If the API rejects: amend the tip (no empty commit) and lease-force the topic branch only
git commit --amend --no-edit
# rearden-grok[bot]: lease-force over HTTPS (rearden-vm-HOST.md)
```

If CI `coverage` fails, add a pin and push. Floor: [`TESTING.md`](../../../TESTING.md).

Operator musl binaries are [`docs/releases.md`](../../../docs/releases.md).
Do not `nix build .#rbitcoin-musl` on a feature branch.
