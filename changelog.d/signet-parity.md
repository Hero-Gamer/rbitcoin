Fixed

- **A signet block with no solution is checked against any challenge.** As
  in Bitcoin Core, a coinbase commitment without the signet section spends
  the challenge with an empty scriptSig and witness, and the challenge
  script decides. Before, such a block was rejected unless the challenge was
  exactly `OP_TRUE`.
