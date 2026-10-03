Fixed

- **BIP30 ignores unspendable outputs.** A repeated txid whose earlier
  instance has every spendable output spent is accepted even when that
  instance also carries an `OP_RETURN` or over-10,000-byte output.
  Bitcoin Core never adds those outputs to the coin view, so they could
  not be overwritten; rbitcoin rejected the block as `bad-txns-BIP30`.
