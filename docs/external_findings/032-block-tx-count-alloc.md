# 032 — Block tx-count allocation

**Severity:** critical
**Status:** fixed
**Found by:** coordinated review, 2026-09-21 (C01) and 2026-09-22

`decode_block_precomputes` read a tx-count `VarInt` and called
`Vec::with_capacity` before decoding any transaction. A count larger than
the bytes left divided by 10 now returns `None`. Ten bytes is the smallest
consensus serialization (version, two compact sizes, locktime).

Store families named with the same shape already bound `n` by a length this
process wrote or checked, and are unchanged:

- `HeaderTxs::get_list` uses `get_range`, which returns `None` when the
  stored count is 0 or above `u32::MAX`.
- `sp_tweaks::read_offs_run` returns `Corrupt` when `local + n` passes
  `n_slots`, before `with_capacity`. `decode_records` takes that same
  header-tx count. The body is already resident; a mismatch returns
  `Corrupt` from the walk. The capacity is the store count, not a length
  read out of the tweak file.
- `InputTable::edges_span` rejects a span past `count()`. Each `n_in` is the
  `u16` already read from that locator slot.
- `scripthash_materialize` phase-done reads require the file length
  `20 + n * 8` and `n == n_shards` before allocating. Worker spans clamp
  `n` to the create-fk span.

**Regression:** `rbitcoin-query` `tx_precompute::tests::decode_block_precomputes_rejects_tx_count_past_payload`,
`tx_precompute::tests::decode_block_precomputes_accepts_minimum_serialized_tx`
