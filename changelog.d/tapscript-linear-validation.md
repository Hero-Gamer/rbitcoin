Fixed

- **Tapscript validation is linear in script, witness, and input count.**
  A leaf with deeply nested `OP_IF`s, many signature checks over a large
  leaf script, annex, or SIGHASH_SINGLE output, or many script-path inputs
  in a tx with large spent scriptPubKeys, cost quadratic CPU to validate,
  so one relayed transaction or block could stall script checks. The IF
  condition stack, the per-input sighash hashes, and the per-tx spent-output
  hashes now follow Bitcoin Core. Accept and reject results are unchanged.
