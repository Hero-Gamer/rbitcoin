# 030 — Header accept and chain wipe

**Severity:** critical
**Status:** fixed
**Found by:** coordinated review, 2026-09-21 and 2026-09-22

A received block whose header fails validation is not inserted or
accepted. A non-genesis block with an all-zero previous hash is not held
and does not disconnect the tip. Header work is summed with a checked
add. A zero `nBits` target is not passed to `Header::work`.

**Regression:** `rbitcoin-net`
`chain::tests::zero_prev_with_live_tip_is_not_held`,
`most_work::tests::sum_work_and_work_better`
