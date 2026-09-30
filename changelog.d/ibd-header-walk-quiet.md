Fixed

- Header look-ahead stops asking a peer after one short reply that does not
  extend the candidate, once checkpoint work meets the floor. A full
  2,000-header window can still build a lighter fork, and a later block
  `inv` puts the peer back on the walk. The 64,000-header refill lane is
  unchanged.
