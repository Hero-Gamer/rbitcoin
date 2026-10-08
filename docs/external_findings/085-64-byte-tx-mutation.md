# 085 — A coinbase-less 64-byte body is mutated

**Severity:** high
**Status:** fixed
**Found by:** consensus audit, 2026-10-02

A merkle root also matches a body made of its inner nodes read as
transactions. A miner grinds a block `[cb, t1]` so that
`txid(cb) || txid(t1)` parses as a 64-byte legacy tx `X`, then sends
`header || 0x01 || X`. `sha256d(X)` is the merkle root, so the merkle
check passed. The tip path then failed `bad-cb-missing` and cached the
hash as invalid. When the real block came, it was refused. A compact
block with that tx prefilled took the same path, after a relay to
high-bandwidth peers.

Core's `IsBlockMutated` treats a block with no coinbase first and any
64-byte (stripped) tx as mutated. It is dropped, not marked failed.
`rbitcoin_consensus::block_mutated_without_coinbase` is that rule. The
P2P `block` handler punishes the sender and drops the body before header,
relay, or accept. Compact reconstruct falls back to a full getdata. The
tip hub returns `NetError::Mutated` for any other caller.

`submitblock` already rejected this body with `bad-cb-missing` before
accept, without a cache. It did cache other mutated bodies (padded
witness bytes); it now skips the cache for every `NetError::is_mutated`.

Block checks compare the merkle root before the other body rules. A first
transaction that is not a coinbase, plus any 64-byte transaction, is
`merkle mutated by a 64-byte tx`. IBD classifies that as a bad copy
(`SoftWire`): the body is dropped, the hash is not cached invalid, and
the block is requested again.

**Regression:** `rbitcoin-net`
`chain::tests::hostile_peer_session`,
`peer::tests::peer_header_dos_and_self_announce`,
`compact::tests::prefilled_64_byte_body_without_coinbase_is_not_a_block`,
`ibd::events::confirm_reject_tests::ibd_mutated_body_is_refetched_not_blacklisted`;
`rbitcoin-rpc` `methods::tests::rpc_regtest_from_genesis`.
