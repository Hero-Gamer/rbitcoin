Changed

- **Class C stores take a single-writer capability.**
  `ClassCWrite` is not `Send`. Confirm-write, tip-accept, and startup mint
  it on the thread that stores. Archived-body clear runs on
  `ibd-confirm-write` after the in-flight tip flush joins. On that path,
  header range puts wait for the same join. Class A body append still
  overlaps the tip flush. A structural reject after that append still
  stores the header ranges after the join, so a retry sees the body.
  Class A device sync stays on `rbtc-spend-sync` about every 10 minutes.
  Class C still flushes once per confirm batch.
- **In-RAM Class C gets take no reader-writer lock.** One sequence counter
  covers a store. The odd bit is sequentially consistent, a release fence
  runs before the slot stores, and the even advance publishes them. Header
  pairs, which have no flush flag, still take a relaxed odd step and that
  same release fence. A get acquire-fences before it checks the counter
  again. `header_txs` reads first and count under one counter. A header
  column load acquires that store, then the same fence checks the counter.
  `strong_tx` keeps its bits in that slot buffer. One flush serves the
  arrays and the bit image. The bit image's length is the body in bytes.
  A flush excludes new stores, then reads the dirty range. The flushing
  flag is sequentially consistent in both directions, with a sequentially
  consistent fence between the odd bit and the flag, so a store cannot
  pass the flag and still be marked clean. The flush does not bump the
  counter. Slot-table growth keeps the previous buffer. A get retries when
  the length it sampled does not fit that buffer. `strong_tx` is one
  anonymous 256 MiB reservation; a grow commits the used prefix and does
  not keep a retired copy.
- **A script-pool claim release-publishes `next` after `in_wave`.**
  `is_complete` acquire-loads `next`, so it does not drop the wave under
  a claim that still applies.
- **Loom and shuttle model the claim order, the idle turn, the UTXO view
  stamp, the Class C sequence counter, and Class C flush exclusion.**
  They are not required PR checks. The index-release order is not a model:
  it needs a `Query` full of `std` locks. The job shape follows phlip9's
  #1214: cfg-gated model tests, a separate target directory, and the same
  pull-request and nightly jobs. This change is an alternative to that
  ArrayTable model. Those scenarios stay on #1214.
