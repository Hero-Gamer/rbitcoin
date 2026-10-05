Changed

- Confirm lookup retires a fence-connected txid after each sealed head
  segment. An unconnected identity still searches later segments.
- Witness txid and sighash midstates are hashed from the block payload.
  BIP143 double-SHA is filled on first use. BIP341 keeps the single SHA-256.
- Lookup decodes the queued block frame in place. Load queue-depth bytes
  are the header, the transaction count, and each transaction's wire length.
