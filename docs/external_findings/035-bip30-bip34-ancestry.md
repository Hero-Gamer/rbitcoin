# 035 — BIP30 follows BIP34 hash ancestry

**Severity:** low
**Status:** fixed
**Found by:** coordinated review, 2026-09-22 (L-9)

BIP30 was skipped whenever BIP34 was active by height. Signet activates
BIP34 at height 1, so a repeated unspent txid was accepted. Core skips
BIP30 only when the block's ancestor at BIP34 height is the network BIP34
hash, and enforces it again from height 1_983_702. Signet and regtest use
a null BIP34 hash, so they always enforce. The two historical mainnet
repeats are unchanged. Regtest's BIP34 height is unchanged.

The txid batch is one `get_fk_by_txid_batch` per checked block, counted in
the structural `spent=` timer. The BIP34 header is one `header_at_height`
read, and only when this network has a BIP34 hash and the block is above
that height.

**Regression:** `rbitcoin-consensus`
`block::structure_rule_tests::buried_rules_and_a_lying_header_path`,
`params::tests::bip34_hash_gates_bip30_like_core`. Finding 019's
`bip30_rejects_unspent_connected_sibling` still rejects.
