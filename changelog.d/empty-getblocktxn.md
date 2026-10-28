Fixed

- **An empty `getblocktxn` disconnects the peer before the block is read.**
  A request with no transaction indexes asks for nothing, but it read the
  whole block and, more than 10 blocks deep, sent it back. As in Core
  (bitcoin/bitcoin#35832), the peer is now disconnected first.
