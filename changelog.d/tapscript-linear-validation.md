Fixed

- **Tapscript validation is linear in script and witness size.** A leaf
  with deeply nested `OP_IF`s, or with many signature checks over a large
  leaf script, annex, or SIGHASH_SINGLE output, cost quadratic CPU to
  validate, so one relayed transaction or block could stall script
  checks. The IF condition stack and the per-input sighash hashes now
  follow Bitcoin Core. Accept and reject results are unchanged.
