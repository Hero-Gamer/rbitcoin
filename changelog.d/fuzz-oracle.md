Added

- Nightly fuzz executes a structured script grammar. Our verifier has a
  100 ms thread-CPU budget, confirmed by a second sample. Seeds come from
  Core `script_tests.json` or the committed fixture when that file is
  absent. A same-hash mutant is replayed as the honest block only after
  both sides reject it. A full compact reconstruct follows the honest body
  through `drain_pending_now` and scores that body against Core without
  invalidating the hash first. P2P sequences are tagged steps. Tx compares
  on the hub mempool. Empty getheaders, feefilter, and inv are not Core
  comparisons; a local drop on feefilter or inv fails the input. Sunday
  runs block-spend under ASan with a 90 s input timeout. `store_reorg`
  drops its hub and reopens the same datadir once per input. The tip hash
  must match. It does not ask Core.
