Changed

- **`estimatesmartfee` returns Core's result shape.** With an estimate:
  `{feerate, blocks}`. Without one: `{errors: ["Insufficient data or no
  feerate found"], blocks}` and no `feerate`, instead of `feerate: -1`.
  The extra `errors: null` and `rbitcoin_model` keys are gone.
  Like Core, `feerate` is at least `mempoolminfee`, so a full mempool
  never gets a rate it would evict.
