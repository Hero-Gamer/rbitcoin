Fixed

- **A torn mempool sidecar no longer stops the node from starting.**
  The 5-second admit persist and shutdown flush sync `tx.body` before
  publishing `LIVE` slots, then sync slots before `meta`. Open keeps
  transactions still inside the logical body and drops the tail. An
  unreadable `meta` / `slots` / `tx.body` (bad magic, unknown schema, or a
  payload that does not decode or whose txid does not match the slot) is
  moved under `mempool/torn-<unix>/` and the node starts with an empty
  mempool. `fee_history` stays in place.
