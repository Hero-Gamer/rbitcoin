Changed

- Confirm load no longer keeps a second scriptSig and witness copy on the
  write plan. Commit encodes those bytes once from the wire transaction and
  the spend edges.
- Script jobs share spent scriptPubKey bytes with the wire block or the
  parent pin instead of copying them into a second output.
