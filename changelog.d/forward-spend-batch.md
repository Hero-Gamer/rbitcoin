Fixed

- **Consensus: a block that spends an output created by a later block is
  rejected in IBD and catch-up.** When several blocks were confirmed in
  one batch, an input could bind to a transaction in a later block of
  that batch. Bitcoin Core connects one block at a time and rejects the
  block with `bad-txns-inputs-missingorspent`; rbitcoin accepted it, and
  a later block could then spend the same output again. The batch is now
  rejected, and the retry names the spending block. A spend slot that
  names a confirmed spender below the output's own height is now a store
  invariant error, not an unspent output.
