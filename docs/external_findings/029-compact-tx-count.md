# 029 — Compact block transaction count

**Severity:** critical
**Status:** fixed
**Found by:** coordinated review, 2026-09-21

`reconstruct` allocated one slot per announced transaction with no ceiling,
and a peer could retain eight of those vectors. The ceiling is
`MAX_BLOCK_WEIGHT / MIN_TX_WEIGHT`. One partial is retained per peer. A
second hash is fetched as a full block and is not ban-scored.
`pending_headers` uses the same 8_000-entry clear on `headers`, `block`,
and `cmpctblock`.

**Regression:** `rbitcoin-net`
`compact::tests::reconstruct_rejects_tx_count_above_weight_ratio`,
`peer::tests::pending_header_insert_past_cap_clears`
