Fixed

- A scripthash pass-1 MPHF without `scripthash.head/NN.packed` is not a durable head. Restart after `DONE.keys` resumes pass 2 and pack instead of reporting Electrum-ready on an index that has no multi-script history. A finished unmarked head is soft-migrated.
