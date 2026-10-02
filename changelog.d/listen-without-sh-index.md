Fixed

- **Electrum and Esplora listen when `--sh-index` is off.** Address and
  scripthash methods still fail closed. With the index on, the listeners
  still wait until scripthash is caught up.
