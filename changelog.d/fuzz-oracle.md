Added

- Nightly fuzz executes a structured script grammar with a 100 ms verify
  budget, replays the honest block after a same-hash mutant, and follows a
  mutated compact body with that honest block through `drain_pending_now`.
  P2P sequences are tagged steps, including inv and feefilter against a
  local node and Core. Sunday runs block-spend under ASan with a 30 s input
  timeout. `store_reorg` reopens its datadir once per input and rejects a
  respend of a spent outpoint on both the reopened hub and Core.
