Fixed

- **Spentness is probed on the connected create.** `gettxout`, mempool accept,
  the proposal check, and the Electrum and Esplora spent checks read the
  spender slots of the row on the best chain; a newer never-connected row for
  the same txid no longer hides a confirmed spend.
