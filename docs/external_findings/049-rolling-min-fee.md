# 049 — Full-mempool fee floor is a static bump

**Severity:** medium
**Status:** fixed
**Found by:** @otaliptus (M-6)

When the mempool was near its weight cap, the minimum feerate was
`minrelay + 100` sat/kvB and did not move when a chunk was evicted. Two
transactions around that static bump could evict each other forever.

Evicting a chunk raises the floor to one sat/kvB above that chunk's
feerate. The static bump remains while the pool is full and no higher
eviction has been recorded. Once the pool is under the cap, the eviction
bump halves every 12 hours until it is back at min relay. There is no
new knob.

**Regression:** `rbitcoin-mempool`
`accept::tests::mempool_under_pressure`,
`accept::tests::relay_floor_decays_when_not_full_and_holds_while_full`.
