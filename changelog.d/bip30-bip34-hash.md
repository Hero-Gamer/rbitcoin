Fixed

- Mainnet BIP30 turns off above height 227931 only when the confirmed header
  there is Bitcoin Core's BIP34 block. The previous constant was a different
  hash, so every later block kept scanning txids for an unspent overwrite.
