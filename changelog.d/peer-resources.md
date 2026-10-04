Security

- Cap the parent-request tracker per peer and process-wide. A full
  process-wide table skips the new announcement and does not disconnect
  the peer. Only a peer at its own cap is disconnected. A wtxid
  announcement is re-requested as a wtxid and does not change another
  peer's txid parent.
- Charge outbound getdata and tx announcements against the per-peer send
  budget, and stop serving blocks once that budget is already over.
- The per-peer rate window keeps the previous second so a boundary does
  not grant a second full budget.
- Count v2 decoy packets and unknown message types in the per-peer rate
  window on tip-follow and IBD. One decoy that does not fit adds the
  rate-limit score. The peer is disconnected at the same threshold as
  other frames.
- Batch mempool transaction announcements into one inv per thousand,
  still charged against the per-peer send budget.
- Findings write-ups: 053, 054, 055, 056, 057, 058, 059.
