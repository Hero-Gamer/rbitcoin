# 047 — Orphan reserve can fill the pool

**Severity:** medium
**Status:** fixed
**Found by:** @otaliptus (M-2)

Two peers can park orphans entirely inside the per-peer weight reserve.
Eviction then skips every entry, so a later orphan is refused and each
refusal rescans every announcer.

The reserve is only the first choice. If nothing outside it can be
dropped, the oldest orphan is evicted and the new one is admitted. Peer
weight is a counter, not a scan. Orphans older than 20 minutes are
removed on insert. Weight, dust, scriptPubKey size, and annex checks run
before park.

**Regression:** `rbitcoin-mempool`
`orphanage::tests::mempool_under_pressure`,
`orphanage::tests::orphan_expires_after_the_bound`.
