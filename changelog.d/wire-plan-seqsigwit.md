Changed

- **Wire confirm encodes scriptSig and witness once.** The write plan no
  longer keeps a second copy. Commit writes those bytes from the wire
  transaction and the spend edges. A records row that arrives with no inputs
  is still corrupt.
- **Script checks borrow spent scriptPubKey bytes.** Same-block spends read
  the wire block on the job. Historical spends read the parent pin. Taproot
  sighash hashes those borrows and does not build a second output.
- **Pruned nodes keep confirmed inputs from the wire transaction.** Connect
  serves the seqsigwit RAM window from that commit cache instead of reading
  each transaction back.
