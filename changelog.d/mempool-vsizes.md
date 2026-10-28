Added

- Mempool RPCs expose `vsize_bip141` and `vsize_adjusted`, with deprecated `vsize` retaining its sigop-adjusted meaning. Verbose `getrawtransaction` adds `vsize_adjusted` for mempool transactions, and `getorphantxs` adds `vsize_bip141` (Bitcoin Core #32800).
