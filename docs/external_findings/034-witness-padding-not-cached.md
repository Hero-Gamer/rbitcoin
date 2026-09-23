# 034 — Witness padding is not the block hash

**Severity:** high
**Status:** fixed
**Found by:** coordinated review, 2026-09-22 (H-3)

Block weight was checked before the witness commitment. Padding a coinbase
witness past the weight limit failed as `block weight too large` and that
failure was cached on the block hash. Witness bytes are not in the hash, so
the unpadded block was then rejected too.

The unexpected-witness and witness-commitment checks now run first. Both are
mutations (`reject_is_mutated`), shared by connect and the two IBD reject
paths. A weight failure after a matching commitment is still cached: the
commitment is in the coinbase txid, so it is this block.

**Regression:** `rbitcoin-net`
`chain::tests::padded_coinbase_witness_over_weight_is_not_cached_invalid`
