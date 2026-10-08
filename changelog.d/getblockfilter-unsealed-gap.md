Fixed

- **`getblockfilter` does not rebuild an unsealed index gap.** A best-chain
  block whose parent filter header is not sealed returns `Index is not caught
  up`. A block whose parent header is already sealed is still built.
