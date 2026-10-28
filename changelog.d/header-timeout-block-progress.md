Fixed

- **IBD header timeouts retain productive block connections without blocking
  frontier handoff.** After repeated late header replies, prefer another header
  peer without discarding useful outstanding downloads when the connection has
  delivered a solicited block within 30 seconds and still has requests on the
  wire. Recent block traffic permits two extra header misses; four consecutive
  unanswered asks disconnect even a productive sole peer. A header reply resets
  the miss count, and the first challenge miss does not change frontier ownership.
  If later bodies are queued and another peer is connected, the retained
  peer's request for the next required block no longer counts as active
  coverage. Its request stays outstanding and its response remains usable;
  the existing scheduler can ask an eligible alternative without waiting for
  the body-queue racing threshold. Other downloads, header validation, body
  budgets, and block-stall handling are unchanged.
