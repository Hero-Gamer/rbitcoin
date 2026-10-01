Changed

- During IBD, confirm does not flush spend annotations or the Class A
  bodies replay reads. `rbtc-spend-sync` syncs those files about every
  10 minutes and records that pre-sync snapshot in `spend_durable`.
  Shutdown does one sync at the latest snapshot before the tip flush.
- Opening a store with no `spend_durable` file revalidates and replays
  spends from genesis. A present file is the cursor open rechecks above.
  A cookie inside a data file, or a clean page cache, is not that cursor.
