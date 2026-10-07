Added

- **SV2 TP build counters.** With `--sv2-tp-listen`, `/metrics` exports
  `rbitcoin_sv2_fee_checks_total`, `rbitcoin_sv2_template_builds_total`,
  and `rbitcoin_sv2_template_build_seconds_total`, and the DEBUG
  `tip: perf` line gains `sv2 checks= builds= build_avg_us= build_max_us=`.
