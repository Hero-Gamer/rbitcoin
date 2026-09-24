# 033 — IBD header path and body intake

**Severity:** critical
**Status:** fixed
**Found by:** coordinated review, 2026-09-21 (C03, C05)

`on_headers_batch` recorded path and explore state before
`ensure_headers_batch`. A batch that fails validation now leaves that
state unchanged. Only the accepted prefix is noted. `explore_need` and
`explore_tips` each keep at most 64 hashes (oldest dropped). Membership
stays a scan at that size.

An unsolicited block frame, and a frame whose hash is already queued, is
dropped before the body-queue copy. Assign will not issue a new getdata
hash when body-queue `bytes()` plus outstanding hashes × 4 MiB
(`GETDATA_RESERVE_BYTES`) would pass `bq_assign_stop_bytes`. A hash that
is already in inflight is still offered. Those two counters are the queue
byte total (`bq RAM=`) and `inflight.len() * 4 MiB`. Offer does not add
a second timer.

**Regression:** `rbitcoin-net`
`events::ibd_memory_tests::rejected_header_batch_does_not_grow_path_or_explore`,
`events::ibd_memory_tests::unsolicited_body_is_not_copied_into_the_queue`,
`reorg::tests::explore_need_and_tips_stay_capped`,
`assign::tests::assign_does_not_issue_when_queue_plus_reserve_exceeds_stop`
