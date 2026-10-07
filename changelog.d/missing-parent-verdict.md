Fixed

- **Consensus: IBD marks a block that spends an unknown txid invalid.**
  A block on the best header chain that spends a transaction the
  connected chain does not have halted IBD as an engine fault, and the
  hash was never marked invalid. Bitcoin Core rejects it with
  `bad-txns-inputs-missingorspent` and follows another valid chain.
  IBD now re-reads the block's parents once the block extends the tip
  and the transaction index covers the chain, and marks the block
  invalid when a parent is still missing. A parent the index lost after
  a power loss is a store fault, not a verdict. A block whose parent
  miss is a store fault goes back for one retry instead of stalling
  IBD, and the check works right after a restart.
