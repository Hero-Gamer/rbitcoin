# 045 — Addr relay fanout

**Severity:** medium
**Status:** fixed
**Found by:** otaliptus review (M-7)

`addrv2` used to forward the whole list to every other live peer. Each
address now goes to one or two neighbors chosen by a stable hash of that
address. A peer may relay 1000 addresses before the bucket must refill at
a tenth of an address per second. One address under that budget still
reaches a neighbor.

**Regression:** `rbitcoin-net` `peer::tests::hostile_peer_session`.
