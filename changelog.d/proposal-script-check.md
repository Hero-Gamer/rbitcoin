Fixed

- **A block proposal with a failing script is rejected.** After the coinbase
  amount check, proposal mode runs the block's script flags on the prevouts
  it already resolved. No second parent decode and no UTXO write. P2SH and
  witness sigops count toward the 80_000 block limit.
