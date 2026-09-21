---
name: overlay-functional
description: >-
  Run or fix the private-mesh overlay harness (Tor chutney-equivalent,
  i2pd netid, cjdns TUN). Use when a PR is labeled overlay-functional, a
  ship version-bump needs that job, or scripts/overlay-functional is red.
  Do not treat this harness as the default PR pin.
---

# Overlay functional harness

Owner: [`docs/overlay-functional.md`](../../../docs/overlay-functional.md).
Default `cargo test` is the PR pin ([`TESTING.md`](../../../TESTING.md)).
Fake SOCKS / control / SAM stay in unlabeled product PRs. Label this
harness and every ship version-bump PR.

When the labeled job is red, reproduce locally. Do not push and wait on
GitHub Actions as the inner loop.

```bash
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target/dev}"
./scripts/overlay-functional/run.sh <filter>
```
