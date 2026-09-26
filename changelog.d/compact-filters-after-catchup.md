Changed

- **`NODE_COMPACT_FILTERS` waits for the filters.** With
  `--block-filter-index`, the node advertises compact filters only once
  its filters first reach the tip, logging `blockfilter: caught up …`.
  Before, it advertised from startup and answered filter requests past
  its progress with silence while the first build ran. `getnetworkinfo`
  now names `COMPACT_FILTERS` in `localservicesnames`.
