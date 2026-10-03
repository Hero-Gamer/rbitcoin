Fixed

- **`gettxout` and `scantxoutset` no longer report the genesis coinbase.**
  Bitcoin Core never adds the genesis block's coinbase output to the UTXO
  set. `gettxout` for that outpoint now returns `null`, REST `getutxos`
  reports it as missing, and `scantxoutset` leaves it out of `unspents`
  and `total_amount`. Electrum and Esplora are unchanged; Esplora matches
  Blockstream/mempool electrs, which index genesis.
