Fixed

- **IBD no longer stalls when a peer re-sends stored headers.** Headers
  the IBD already stored keep the store row they were accepted with.
  Checking them again walked each one back to the connected tip, and a
  fresh mainnet sync from one peer stopped after 16 blocks.
