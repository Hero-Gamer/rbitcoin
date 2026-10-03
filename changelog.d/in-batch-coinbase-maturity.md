Fixed

- **Coinbase maturity holds inside one confirm batch.** When a coinbase and
  a transaction that spends it were in different blocks of the same write
  batch (IBD, catch-up, or reorg connect), the 100-block maturity check
  did not run, so a block that Bitcoin Core rejects with
  `bad-txns-premature-spend-of-coinbase` was accepted. The batch now marks
  each block's coinbase itself instead of reading `confirmed`, which is
  not yet written for those heights.
