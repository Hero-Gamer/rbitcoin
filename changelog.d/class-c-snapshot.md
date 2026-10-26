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
