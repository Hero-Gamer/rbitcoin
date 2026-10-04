Fixed

- **Index build refuses a parent or witness span past the published body.**
  The block's own txout span was checked against the file end. A parent
  txout, or a seqsigwit span for a P2TR output, was still read from the
  slab after that end had moved backward.
