Changed

- **Mempool accept decodes a confirmed parent once per coin lookup.** Resolving
  a spent coin reads the parent's packed outputs once; the fk resolve verifies
  `txid.body` only and coinbase-ness comes from the block's first tx, not a
  second decode of the parent's inputs. The `getblocktemplate` proposal check
  shares that lookup.
