---
name: release
description: >-
  Cut a minor, patch, or major release, or tag a GitHub Release. Use when
  the user says do a minor release, do a patch release, do a major release,
  or asks to tag vX.Y.Z. The playbook owner is docs/releases.md; do not
  copy it.
---

# Release

Follow [`docs/releases.md`](../../../docs/releases.md). That file is the only
playbook (changelog cut, Highlights, tag, `vX.Y.x`, `.99` bump). Do not copy
it into this skill.

Ship commands (worktree, HTTPS push, poll) are
[`ship-pr`](../ship-pr/SKILL.md). A ship PR also needs the `release`
label and a green `release-extra` check (core, overlay, and Warnet
functional all green).
See the CI gates section of `docs/releases.md`.

## Scripts

| Command | Does |
|---------|------|
| `./scripts/release-cut.sh --minor` | `X.Y.99` → `X.(Y+1).0`; cuts CHANGELOG |
| `./scripts/release-cut.sh --major` | `X.Y.99` → `(X+1).0.0` |
| `./scripts/release-cut.sh --patch` | next patch on `vX.Y.x` |
| `./scripts/release-cut.sh --dev-next` | just-shipped `X.Y.0` → `X.Y.99` |
| `./scripts/release-gate.sh` | cargo / nix / changelog; ship needs Highlights |
| `./scripts/release-notes.sh` | GitHub Release text |
| `./scripts/release.sh` | annotated tag on a ship version |
| `./scripts/release-post.sh` | tag, and for `X.Y.0` create `vX.Y.x` |

Hermetic pins: `./scripts/release.test.sh`. Prefer `--no-push` on this VM,
then HTTPS-push the tag (a `GITHUB_TOKEN` tag push does not start
`release.yml`).

Which sequence to run (minor, patch, major) is the Playbooks section of
[`docs/releases.md`](../../../docs/releases.md). Stop before tagging `v1.0.0`
if [`docs/road-to-1.0.md`](../../../docs/road-to-1.0.md) still has an open
promise.
