Fixed

- **Mainnet no longer warns `Unknown new rules activated (versionbit 0)`,
  `(versionbit 1)` and `(versionbit 2)`.** The unknown-bit check counted
  blocks from genesis, so the CSV, SegWit and Taproot signalling periods
  looked like unknown rules. As in Core, blocks below `MinBIP9WarningHeight`
  (711,648 on mainnet, 2,013,984 on testnet3) no longer count, and mainnet
  needs 1815 of 2016 blocks (90%) instead of 1512.
