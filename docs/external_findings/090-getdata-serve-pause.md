# 090 — Served getdata past the writer queue was dropped

**Severity:** medium — P2P liveness
**Status:** fixed
**Found by:** CI flake triage (`end_of_ibd_follow`), 2026-10-07

Core `ProcessGetData` stops when the send buffer is full (`fPauseSend`)
and keeps the rest of the request for a later pass. It does not drop a
requested hash.

rbitcoin served a `getdata` one item at a time and dropped items in two
cases: when 16 (`MAX_SERVE_BLOCKS`) served bodies were already in the
session's outbound queue, and when that queue was past the 4 MiB send
budget ([043](./043-peer-send-buffer.md)). It sent no `block` and no
`notfound` for a dropped hash. The requester waits for the hash until
its stall timer fires. rbitcoin IBD waits 30 s, then disconnects the
honest peer and puts its address in a cooldown.

rbitcoin IBD asks one peer for up to 64 blocks
(`DEFAULT_BLOCKS_IN_TRANSIT_PER_PEER`). When the serving node's writer
fell behind, it dropped part of the request. Core asks one peer for up
to 16 blocks in one `getdata`. After three 1.5 MB blocks the queue is
past the send budget, and the rest of that `getdata` was dropped.

Repro: `rbitcoin-test` `end_of_ibd_follow`, four copies pinned to one
CPU. 10 of 24 runs held a fetch hole (`ibd: progress … hole=N`) past
the 30 s listener wait. In each traced run, the serving node dropped
exactly `N` hashes. With this fix, 0 of 24 runs stalled.

**Fix:** the session serves `getdata` items in order while its writer is
under the send budget and holds fewer than 16 served bodies. When it has
no room, it sends the `notfound` it collected, keeps the rest of the
inventory on the session, and returns to its loop. It reads no new
message until that tail is served, as Core does not process a peer's next
message while its getdata queue is non-empty. The heartbeat still runs,
so a peer that never reads hits the ping timeout. The tail resumes when
the writer frees room, and that wait ends when the session is told to
disconnect or its writer is gone. The writer frees the served-body slot
before it wakes the session.

Only a getdata body takes a slot, and only it frees one. A compact tip
announce or a deep `getblocktxn` block used to free a slot it never
took, which let more than 16 small bodies queue behind a paused getdata.

The RAM bound: one session queues at most 16 reconstructed bodies, and
holds one paused inventory of at most 50,000 items (about 2 MB).

**Regression:** `rbitcoin-net`
`peer::tests::getdata_past_serve_cap_waits_for_writer`,
`peer::tests::getdata_over_send_budget_waits_for_writer`,
`peer::tests::paused_getdata_ends_on_disconnect_or_dead_writer`,
`peer::tests::paused_session_still_times_out_a_silent_peer`,
`peer::tests::paused_getdata_tail_is_served_before_the_next_getdata`,
`peer::tests::uncounted_bodies_do_not_open_serve_slots`
