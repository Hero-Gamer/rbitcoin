Fixed

- A scripthash pass-1 MPHF without `scripthash.head/NN.packed` is not a durable head. Restart after `DONE.keys` resumes pass 2 and pack instead of reporting Electrum-ready on an index that has no multi-script history. A finished unmarked head is soft-migrated.
- Electrum stays down when a durable scripthash head's inclusion floor is behind the tip, and the same process binds it once write-behind catches up. A cancelled extract names `scripthash.cold_progress` or `scripthash.unsorted` only when that path is on disk.
- A tip append onto an unsealed pass-1 head no longer hides the packed multi-script chain. Packing the shard drops that ingest row once main owns the key.
