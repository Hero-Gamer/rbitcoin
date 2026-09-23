# 043 — Per-peer outbound queue

**Severity:** high
**Status:** fixed
**Found by:** coordinated review, 2026-09-22 (gist H-4)

Each live session keeps an outbound byte estimate. Headers count as 81
bytes, inv and notfound rows as 36, addr rows as 30, and transactions and
blocks as their total size. Once that estimate is past 4 MiB, the session
does not serve another inbound request until the writer drains. A single
reply may cross the cap. Full-block serves that already fit
`MAX_SERVE_BLOCKS` are still queued; their bytes count. `getaddr` is
answered once per connection.

**Regression:** `rbitcoin-net` `peer::tests::getheaders_flood_stops_at_the_send_budget_and_getaddr_is_once`.
