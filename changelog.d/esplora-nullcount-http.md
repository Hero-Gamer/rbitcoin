Fixed

- **Esplora paged address history is served above `--max-sh-creates`.**
  `/txs`, `/txs/chain`, and `/txs/summary` return their page when a script
  has more creates than the cap. Unpaged stats and `/utxo` still return 503.
- **`/txs/summary` rows include `tx_position`**, and `?asof=` is accepted on
  that route. The page stays 25 rows.
- **Esplora tx JSON omits an empty `witness`.** `inner_redeemscript_asm` is
  only the P2SH redeem script, and `inner_witnessscript_asm` is only a
  P2WSH witness script or a Taproot script-path leaf.
- **`/block/:hash/txs/:start` at or past the last tx is 404**
  `start index out of range`.
