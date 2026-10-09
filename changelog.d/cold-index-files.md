Changed

- `--datadir-cold` is the append-only volume: `seqsigwit`, `txstat`,
  `input`, and, once enabled, `blockfilter` and `sp_tweaks`.
  `--prune-seqsigwit` still only drops historical seqsigwit and keeps
  its 288-height window on the hot store. A split datadir that still
  has `blockfilter.*` or `sp_tweaks.*` on the hot store refuses to open
  until those directories are moved next to seqsigwit.
