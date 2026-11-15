Fixed

- **A restarted node no longer builds blocks that spend a missing parent.**
  After an unclean stop within about 5 s of a reorg, the persisted mempool
  could reload a child whose parent was on neither the active chain nor in
  the pool. `getblocktemplate` and the SV2 template provider then offered
  that child, every block built from them was rejected with
  `bad-txns-inputs-missingorspent`, and `generatetoaddress` failed until the
  child expired. Tip-mode entry now drops such a transaction and its
  descendants, as Bitcoin Core's reload does (`missing-inputs`).
