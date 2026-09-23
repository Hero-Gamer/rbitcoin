# 038 — Async tip-accept lifetime

**Severity:** high
**Status:** fixed
**Found by:** coordinated review, 2026-09-21 (C11)

The async tip-accept job is `'static`. It does not extend a borrow with a
lifetime transmute. Dropping the session future does not join the job and
does not leave that job running against a freed `ChainHub`: the job holds
an `Arc` taken from [`ChainHub::into_arc`]. A stack hub used by unit tests
still joins on the synchronous lane, which returns only after the job
finishes.

Process shutdown waits until that lane is idle before the store flush.
Dropping a `P2PNode` still aborts the connect-retry task and returns.

**Regression:** `rbitcoin-net` `tip_accept::tests::owned_job_finishes_after_waiter_abort`,
`chain::tests::into_arc_shares_one_hub`,
`chain::tests::accept_received_block_async_connects_off_worker`,
`service::tests::drop_does_not_pin_and_hub_is_shared`.
