# 027 — Script skip keyed by txid

**Severity:** critical
**Status:** fixed
**Found by:** coordinated review, 2026-09-21 and 2026-09-22

Tip confirm skipped script checks for a block transaction whose txid was
already in the mempool. Segwit txid does not commit to the witness, so a
different witness shared that skip. The skip now requires the mempool
entry's wtxid to match the block transaction.

**Regression:** `rbitcoin-net`
`tx_relay::tests::tip_script_pres_skips_only_matching_wtxid`
