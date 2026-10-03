Fixed

- **Regtest enforces BIP34 from height 1, like Bitcoin Core.** Regtest kept
  rust-bitcoin's BIP34 height of 100000000, so a coinbase without the block
  height push was accepted where Core rejects it.
  `-testactivationheight=bip34@N` still moves the height. BIP30 stays
  enforced on regtest, because regtest has no BIP34 hash.
