Fixed

- **`getrawtransaction` honors `blockhash` and verbosity 2.** The tx is
  returned only when that block contains it, with `in_active_chain`.
  Verbosity 2 on a confirmed non-coinbase adds a BTC `fee` and
  `vin[].prevout`. An unknown txid uses Core's
  `Use gettransaction for wallet transactions.` suffix. A pruned block
  that does not contain the tx is still that `-5`, not the pruned `-8`.
  An inactive block reports `confirmations` `0` and omits `time` and
  `blocktime`.
