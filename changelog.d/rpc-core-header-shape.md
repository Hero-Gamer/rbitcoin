Fixed

- **Block header and block RPC match Bitcoin Core's JSON for inactive
  headers, genesis, and difficulty text.** `getblockheader` and `getblock`
  return a stored header that is no longer on the active chain
  (`confirmations: -1`, its own `previousblockhash`, and a Class A body when
  one exists). Genesis omits `previousblockhash`. `getblockheader` includes
  `nextblockhash` and `target`. `getblockchaininfo` includes `bits` and
  `target`. Difficulty uses 16 significant digits. `validateaddress` reports
  `isscript` for P2TR and P2A and omits witness fields on pay-to-anchor.
  `getmempoolinfo` includes `fullrbf`, `maxdatacarriersize` (null), and the
  cluster limits. BTC amounts print as 8-decimal numbers.
