Changed

- **IBD and tip perf lines are one JSON object.** DEBUG `ibd: perf`,
  `tip: perf`, and `tip: accept` each print a timestamped JSON object
  (`ts` is unix milliseconds, zeros included) instead of hand-written
  tokens. `ibd: sizes` and `ibd: perf_dbg` are gone; those counters are
  fields on the `ibd: perf` object. `ibd: progress` and `tip: best` are
  unchanged. A full IBD is every `ibd: perf` line in a debug log.
