# Errata

Known limitations that are **not** treated as current must-fix bugs.
Durable confirm still uses TipOnly (connected instance). See
[`invariants.md`](./invariants.md) leftover identity and
[`017-duplicate-txid-unconnected-instance.md`](./external_findings/017-duplicate-txid-unconnected-instance.md).

## RAM leftover maps: one fk per txid

In-flight `creates` and pipeline pin `by_txid` are `txid → one fk` (last
write). A second `create_fk` for the same txid **clobbers**
the first in that map. Do not stall the pipeline on a second fk (forget
cannot run).

**Pre-BIP30:** clobber is correct enough. The maps are a thin write-behind
/ pipeline identity, not a BIP30 instance list. Newest/last entry is an
acceptable single identity; durable TipOnly still prefers connected.

Stamp/load may bind a parent `create_fk` from in-flight that is **ahead of**
or **not** the write-batch’s create (same txid at two live pack heights, or
a spend whose in-flight identity is the later overwrite). Write rejects that
bind fail-closed (spentness / missing create / `SpansTip`). That is not a
fork.

**Post-BIP30:** a second *connected* create of the same txid is invalid.
The only realistic overlap is a **disconnected** Class A sibling (reorg,
same tx on the new tip) still sitting in a RAM map while the new fk is
noted. Last-write could hide one of them — a *possible* identity
visibility hole. Unlikely: both rows stay on disk; TipOnly still picks
connected; n−1 is held in in-flight until after the child pin.

Do not grow these maps to `Vec<Fk>` unless a mainnet miss is shown to
be this case.

## Retired confirm dual paths

These names are gone. A missed fact the pipeline promised is still
`StoreError::Corrupt("invariant: …")` ([`invariants.md`](./invariants.md)).
Do not treat the list as a set of identifiers to police:

- Soft spentness recovery for a wrong or missing pin identity.
- Unpinned wire-corrected `create_fk` spentness.
- Load-stage `txid.body` identity fill after lookup promised the stamp.
- `ColdPinMode` Allow/Forbid cold denserels on load (load is range outs only).
- Denserels-as-spender-abs (schema 22 abs is `spent` loc off + `8×vout` only).
- `AssembleMode::Full` / `validate_block_connect` (confirm is optimistic
  assemble, then `structural_validate_spends`).
- `archive_plan_batch_from_store` and production `Query` turning TxApply into
  a dummy `Block`. `tx_apply_to_tx`, `connect_block`, and
  `commit_class_a_only` are `rbitcoin_query::testutil::FixtureChain` only.
