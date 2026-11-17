Changed

- **The SV2 template provider builds on the November 2026 Stratum crates.**
  `binary_sv2` 8.0.0, `codec_sv2` 8.1.0, `common_messages_sv2` 10.0.0, and
  `template_distribution_sv2` 8.0.0 replace the September set. The bytes on
  the wire are the same. The crates follow the spec's field names, so the
  log line for a `SubmitSolution` outside the rolling time window now reads
  `ntime` where it read `header_timestamp`. The AES-GCM crates leave the
  build: `buffer_sv2` 4.0.0 no longer depends on them.
