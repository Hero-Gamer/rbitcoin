Fixed

- **A reorg puts its disconnected transactions back on the new tip.** A
  reorg that replaced two or more blocks re-added their transactions on the
  fork point, before the new branch connected. A Core wallet sets a
  transaction's lock time to the height it last saw, so a transaction
  confirmed in the second disconnected block was not final there and was
  lost from the mempool and the orphanage. As in Core, the transactions go
  back once the new branch is connected, oldest block first. A transaction
  whose parent does not come back is dropped and does not wait in the
  orphanage (#1236).
