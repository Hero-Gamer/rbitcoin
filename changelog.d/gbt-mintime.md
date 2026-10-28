Fixed

- **`getblocktemplate` `mintime` is one past the tip's median time.** It was
  the median itself, which a block cannot use (`time-too-old`). The Esplora
  `GET /block-template` JSON follows.
