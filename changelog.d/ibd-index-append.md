Changed

- **Block filters and silent-payment tweaks are sealed during sync when
  they are enabled before it.** `--block-filter-index` and `--sp-tweaks`
  append each connected batch on the confirm write thread. A restart gap of
  at most one write drain is sealed at startup. Turning either index on
  after those blocks were connected still builds the gap after catch-up.
