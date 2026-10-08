Fixed

- **A lower `--min-relay-tx-fee` is the fee peers are told.** The rolling floor starts at the configured minimum. It no longer stays at the default 100 sat/kvB, so the BIP133 feefilter drops with `-minrelaytxfee` once the node leaves IBD. An eviction that already raised the floor still holds.
