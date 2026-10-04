Security

- `/rest/` on the RPC listener is off unless `--rest` or `rest=` is set.
  It uses its own queue, and the body is read before that permit is taken.
- A silent-payment subscribe scans at most the recent 256-block window,
  including when the client passes a start height. The scan stops when
  the client hangs up.
- RPC waits are capped at two minutes inside the handler, so a long-poll
  that hits the cap still returns a JSON-RPC body. The listener drops a
  new connection once 256 are open. A long-poll does not hold a
  work-queue slot.
- API logs strip `xprv` / `tprv` material and silent-payment scan secrets.
- Findings write-ups: 063, 064, 065, 066.
