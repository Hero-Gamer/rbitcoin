Fixed

- **After catch-up, outbound full-relay peers fill up to `--max-outbound`.**
  The first tip-follow dial is no longer a one-shot of at most 3, and a
  shortfall is refilled on the connect-redial interval, including when
  `--seed-node` learns addresses into an empty book. `--trusted` still
  fills that quota. A full quota still rotates one outbound full-relay
  peer when the tip is stale and seeds are enabled. `--connect` and
  `--trust-peer` stay pinned to those addresses.
