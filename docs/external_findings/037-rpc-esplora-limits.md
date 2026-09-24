# 037 — RPC body, work queue, waits, and Esplora client id

**Severity:** high
**Status:** fixed
**Found by:** coordinated review, 2026-09-21 and 2026-09-22 (C12, C13, and the authenticated-RPC and loopback-client notes)

HTTP JSON-RPC checks the bearer on the request parts before the body is
read. A missing or wrong token is 401 with no body bytes required. The
body cap is the named `RPC_MAX_HTTP_BODY` (2 MiB): a larger body with a
valid bearer is 413 and the method does not run. A body under that cap
still dispatches.

`--rpc-work-queue` defaults to 16, matching Core `-rpcworkqueue`. A full
queue is HTTP 503. **0** is that same default queue.

`waitforblock`, `waitforblockheight`, `waitfornewblock`, and
`getblocktemplate` longpoll wait on the async runtime (tip broadcast, the
method timeout, or stop). The blocking pool only performs the cheap read
after that wait, so one long-poll does not occupy a blocking thread for
the whole interval.

Esplora uses `X-Rbitcoin-Client` as a join-cache key only for a unix
socket or when join-header trust is set. The public listener does not
read the header. Loopback alone does not.
`SECURITY.md` names the REST cap as concurrent requests, not an
accepted-socket cap.

**Regression:** `rbitcoin-rpc` `server::tests::unauthorized_large_content_length_is_401_before_body`,
`server::tests::oversized_bearer_body_is_413_and_small_body_still_runs`,
`server::tests::waitfor_does_not_hold_the_blocking_pool`,
`rbitcoin-node` `config::tests::rpc_work_queue_zero_is_the_default_finite_queue`,
`rbitcoin-esplora` `server::tests::http_sh_join_last1_last_bulk_and_header_trust`.
