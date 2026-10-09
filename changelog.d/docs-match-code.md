Fixed

- **Operator and schema docs match the node.** Blocks in transit per peer
  are 64. `--mempool-size-mb` is N × 1_000_000 weight units. Scripthash
  extract workers are one per 1.5 GiB. Confirm commits the height fence
  and `confirmed[]` before scripthash write-behind. Schema file headers
  are version 26, including block-filter kind 24 and `input.*`.
- **IBD and mempool operator notes match the code.** Densify is 32
  hashes per peer (64 for a fast outlier). Cluster caps are 64 txs and
  101 kvB. The mempool sidecar is schema 3. `tip: accept` is DEBUG.
  Linux falls back to pread when io_uring cannot open.
- **CLI names in the product docs are `--sh-index` and `--sp-tweaks`.**
  Electrum and Esplora start with the scripthash index off. `generate*`
  also runs on an `OP_TRUE` signet. Electrum `protocol_max` is 1.6.
  SV2 template-provider plans A–C are landed. CLN and ldk-node chain
  backends are the contract in `lightning.md` (Q-69 closed).
