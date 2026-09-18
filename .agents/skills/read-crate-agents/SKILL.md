---
name: read-crate-agents
description: >-
  Read the crate-local agent index before editing code under crates/. Use
  when the task changes rbitcoin-store, rbitcoin-query, rbitcoin-net,
  rbitcoin-consensus, rbitcoin-rpc, or any other crate that has an AGENTS.md.
---

# Read the crate index

Before the first edit under `crates/<name>/`, open `crates/<name>/AGENTS.md`
when that file exists. It names neighbors, owner docs, and the default
`cargo test -p` command. It is an index, not a second design book.

If the crate has no `AGENTS.md`, use [`docs/ORIENT.md`](../../../docs/ORIENT.md)
and the owner linked from [`docs/README.md`](../../../docs/README.md).
