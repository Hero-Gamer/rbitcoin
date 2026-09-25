Changed

- **`--block-filter-index` no longer runs during IBD.** Basic filters
  follow the scripthash lifecycle: IBD and catch-up build none; after
  catch-up a background appender materializes the gap from stored chain
  data (logging `blockfilter: materialize` progress), then seals each new
  block. Follow, relay, and Electrum start without waiting for it, and
  shutdown during the materialize is prompt; the next start resumes.
  Enabling the index on an existing datadir no longer stalls IBD exit.
- **Filters build on `--prune-seqsigwit` nodes.** They read output and
  spent-prevout scripts from stored chain data, not reconstructed blocks.
- **Filter storage is indexed.** Any height's filter is two reads; filter
  hashes and headers are one. Writes are crash-consistent, and on open
  slots for blocks no longer on the best chain are dropped.
- **BIP157 serving matches Core on bad ranges.** `getcfilters` /
  `getcfheaders` with start past stop, or more than 1000 / 2000 heights,
  disconnect the peer instead of returning a clamped batch.
- **`tip: accept`** logs `bf=` (filter appender time) and `bf_lag=`.
