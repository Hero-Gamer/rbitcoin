# 039 — Store I/O buffer lifetime

**Severity:** high
**Status:** fixed
**Found by:** coordinated review, 2026-09-21 and 2026-09-22 (Third #11, L-6)

`UringSession::push_pread` and `push_pwrite` (and the flag variants) are
`unsafe`. The buffer must stay live until the completion is harvested or
`drain_all` returns. A safe caller cannot submit and then free the buffer.

`HeadDrainHandle` borrows the `Store` until join. Dropping the handle
still waits for the insert. `mem::forget` on the handle is not a
supported way to skip that wait.

An `io_uring` enter failure with SQEs still pending takes the same path
as the drain hard cap (process abort outside tests) so the caller does
not free those buffers. A failure with nothing in flight still returns
the enter error.

The script-pool `Relaxed` counter note from the same review is won't-fix:
no failing test showed a reorder, and the counters are statistics.

**Regression:** `rbitcoin-store` `uring_session::tests::enter_failure_with_pending_matches_the_hard_cap`,
`rbitcoin-consensus` `confirm_run::write_idempotent_tests` head-insert join.
