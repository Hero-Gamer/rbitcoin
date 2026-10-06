# 087 — A spend of a later block in one confirm batch was accepted

**Severity:** critical — consensus split
**Status:** fixed
**Found by:** consensus audit, 2026-10-06 (two independent regtest repros)

Core connects blocks one at a time. When block N is connected, the coin
view holds outputs of blocks below N and of earlier transactions in N.
A transaction in N that spends an output first created in block N+1
fails `CheckTxInputs` with `bad-txns-inputs-missingorspent`.

IBD and catch-up confirm several blocks in one batch. The archive plan
binds every input through a txid map over all transactions of the batch
(`rbitcoin-query` `archive.rs`), and the already-archived load path does
the same (`confirm_run/lookup.rs` `same_batch`). Neither map checks the
order of blocks. Assemble then found the parent in the batch pin, and
structural read its create height but checked only coinbase maturity.
[005](./005-non-topological-block-accepted.md) closed the same hole for
two transactions in one block; the order check it added is per block.

Repro on regtest: mine 101 blocks; `T` spends the coinbase of block 1;
`U` (version 1) spends `T:0`. `confirm_wire_run` of `[102: U, 103: T]`
in one call returned `Ok`, and the tip moved to 103. Core rejects block
102. A later block that spends `T:0` again was also accepted: structural
read the spender of `T:0` (`U`, height 102) as lower than the create
height (103) and treated the output as unspent. The tip path connects
one block per call and was not affected.

**Fix:** structural create heights reject an input whose parent is
created in a later block of the batch as `MissingPrevout`. Every bound
parent of every spend passes through this check, for the plan path and
for the archived path, before the batch writes Class C. The batch is
not connected, and IBD retries it one block at a time, so the reject
names block N.

On that retry, block N is archived and `T` is archived but not
connected, so the lookup cannot bind `U`'s input. The archived lookup
without a skeleton (tip path, `confirm_wire_run`) left that input
unbound, and assemble failed with a store invariant. It now rejects the
block as `MissingPrevout`, as the plan path does.

A confirmed spender at a height below its create height is now
`StoreError::Corrupt("invariant: …")`. That branch was added in 2026-08
to step over spend slots that old tip annotate bugs wrote. Schema 22
refuses every datadir from before those fixes, and a valid chain
cannot write such a slot. Treating it as unspent hid this bug.

**Known gap:** in IBD, a missing prevout at the lookup stage is still
an engine fault (requeue once, then halt), not a block verdict. This
is the existing rule for any block that spends an unknown txid; this
fix does not change it. The block is never connected.

**Regression:** consensus matrix row C42;
`rbitcoin-test` `consensus_rules::same_batch_spend_of_a_later_block_is_missing`;
`rbitcoin-net` `ibd::events::confirm_reject_tests::batched_spend_of_a_later_block_is_rejected_alone`;
`rbitcoin-consensus`
`confirm_run::spend_durable_tests::spender_below_its_create_height_is_corrupt`
