Fixed

- **`getblockfilter` does not rebuild an unsealed index gap.** A best-chain
  block whose parent filter header is not sealed returns “Filter not found.
  Block filters are still in the process of being indexed.” A block whose
  parent header is already sealed is still built, and a stale branch is
  rebuilt only back to that sealed fork point.
