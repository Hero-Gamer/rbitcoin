Changed

- Confirm load no longer keeps a second scriptSig and witness copy on the
  write plan. Commit encodes those bytes once from the wire transaction and
  the spend edges.
