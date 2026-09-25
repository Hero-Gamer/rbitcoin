# 046 — Spend annotations lost after the tip seal

**Severity:** high
**Status:** fixed
**Found by:** @otaliptus (H-5, H-6, L-5)

`flush_class_c_tip` makes the tip durable and does not `sync_data` the
spend stems. A missing spent slot is treated as unspent, so a later block
can spend an output the sealed tip already spent.

Open replays `finish_post_commit_hashes` for heights above the
`spend_durable` marker, then `sync_data`s the stems that replay reads and
publishes the marker. A missing marker checks the last 6 blocks. Matching
spends publish the marker at the tip. A mismatch replays from genesis. The write thread does that sync every 8 confirm
batches or 30 seconds. The tip-window check stays at least 6 blocks wide
and includes every height above durable-through. Disconnect below the
marker lowers it. `tip_seal`, `tx.head` meta, and the marker `fsync` the
parent directory after tmp+rename.

**Regression:** `rbitcoin-consensus`
`confirm_run::spend_durable_tests::zeroed_spend_slot_after_tip_seal_rejects_respend`,
`rbitcoin-store`
`integrity::tests::durable_through_widens_past_the_six_block_window`.
