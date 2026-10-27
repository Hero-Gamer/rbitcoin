Fixed

- **LND and other btcd `rpcclient` users can broadcast.** That client reads
  `/rbitcoin:` as Core before 0.19 and sends `sendrawtransaction` a boolean
  `allowhighfees`, which was `-3 maxfeerate is not a number or string`.
  `true` now lifts the fee cap and `false` keeps the default 10000 sat/vB.
