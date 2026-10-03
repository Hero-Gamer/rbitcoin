Fixed

- **A signet block with no solution is checked against any challenge.** As
  in Bitcoin Core, a coinbase commitment without the signet section spends
  the challenge with an empty scriptSig and witness, and the challenge
  script decides. Before, such a block was rejected unless the challenge was
  exactly `OP_TRUE`.
- **The signet commitment rewrite matches Bitcoin Core byte for byte.** A
  zero-length `OP_PUSHDATA1/2/4` in the witness commitment is kept as its
  bare opcode, and a push longer than 65535 bytes is re-encoded with
  `OP_PUSHDATA4`. Before, the empty push became `OP_0` and the long push got
  a truncated `OP_PUSHDATA2` length, so the modified merkle root and the
  signet signature hash differed from Core.
