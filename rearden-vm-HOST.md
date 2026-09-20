# rearden-grok operator VM

**Ignore this file unless you are acting as `rearden-grok[bot]`.**

This is one operator Cursor VM (ephemeral disk, no App SSH key). It is not
contributor setup, CI, a fork, or any other agent identity. Humans and
other agents use [`TESTING.md`](TESTING.md) Artifact silos and
[`CONTRIBUTING.md`](CONTRIBUTING.md) (`$PWD/target/dev`).

Owner of these host facts. Do not copy the tables into `AGENTS.md`,
`TESTING.md`, or the skills.

## Identity

Worktree-only (do not change the Cursor checkout `user.name`):

```bash
~/.config/rbitcoin-grok/configure-worktree.sh .
# rearden-grok[bot] <317016512+rearden-grok[bot]@users.noreply.github.com>
```

Re-auth (~1 h token): `~/.config/rbitcoin-grok/gh-login.sh`.

## Disk and cargo silo

Root is ~40 G. Git worktrees already share `.git` objects. They do **not**
share `target/dev` (~9 G warm) or a `third_party/bitcoin` working copy.
Extra worktrees and cargo in the editor tree are what fill the disk.

| Share | How |
|-------|-----|
| One session worktree | `/tmp/rbtc-<session>` for the session. Next PR is `git switch -C`, not `git worktree add`. |
| One dev silo | Export **`CARGO_TARGET_DIR=/tmp/rbtc-target/dev`** *before* `nix-shell` / `nix develop` (the hook only sets `$PWD/target/dev` when unset). Registry/git deps occupy disk once; workspace crates re-fingerprint if the source path changes. |
| Editor tree | `/home/agent/workspace/rearden-bitcoin` is the Cursor checkout. Do not cargo there. Do not `git worktree add` from it for a second PR. |
| One cargo at a time | A second cargo **waits** on `target/.cargo-lock` (serialized, not a torn rlib). Do not `cargo clean` under another cargo. Unhashed binaries (`debug/rbitcoin-node`) are last-writer-wins. |
| ENOSPC | Skip production-scale body tests (`sp_tweaks` and similar multi‑GiB `/tmp` files). Skip `cargo test --workspace` when free space is a few GiB. Never `./scripts/coverage.sh` here (`target/cov` is another silo). Targeted `-p` tests plus clippy are enough to push. |
| Session end | `git worktree remove` leftover `/tmp/rbtc-*`. `rm -rf /tmp/rbitcoin-*` test dirs. Do not `cargo clean` the shared silo unless artifacts are stale. |

Do not change `shell.nix` / `flake.nix` defaults to `/tmp/rbtc-target/dev`.
Those stay `$PWD/target/dev` for humans and CI.

## Session worktree

```bash
# once per session
git fetch origin
git worktree add /tmp/rbtc-<session> origin/master
cd /tmp/rbtc-<session>
git switch -c <area>/<short-name>
export CARGO_TARGET_DIR=/tmp/rbtc-target/dev
~/.config/rbitcoin-grok/configure-worktree.sh .

# next PR in the same session (do not add another worktree)
git fetch origin
git switch -C <area>/<next-short> origin/master
```

```bash
# session end
git worktree remove /tmp/rbtc-<session>
rm -rf /tmp/rbitcoin-*
git fetch origin --prune
```

Commands after that: [`.agents/skills/ship-pr/SKILL.md`](.agents/skills/ship-pr/SKILL.md).
Cargo/clippy/deny/rustc stdout: [`docs/how-we-plan.md`](docs/how-we-plan.md)
(Agent RAM).

## Fetch and push

`origin` stays SSH (`git@github.com:reardencode/rbitcoin.git`) with HTTPS
fetch. This VM has no App SSH key (`ssh -T git@github.com` → Permission
denied). Never `git remote set-url origin` (worktrees share remotes).

Bot `git fetch` / `git push` use an explicit HTTPS URL. Do not
`git push origin`. No `-u` (that retargets the branch remote away from
`origin`).

```bash
~/.config/rbitcoin-grok/gh-login.sh
git fetch origin
git push https://github.com/reardencode/rbitcoin.git HEAD:<area>/<short-name>
gh pr create --repo reardencode/rbitcoin --head <area>/<short-name> --title "…" --body "…"

# rebase then update the topic branch
git rebase origin/master
git push --force-with-lease https://github.com/reardencode/rbitcoin.git HEAD:<area>/<short-name>

# after merge, delete the topic branch only
git push https://github.com/reardencode/rbitcoin.git --delete <area>/<merged-short>
```
