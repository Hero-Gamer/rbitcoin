Fixed

- **A replacement that pays the incremental minimum is no longer rejected for sharing a truncated sat/kvB bucket.** RBF requires a strictly higher true feerate.
- **Advertised minimums match the fee the mempool enforces.** `getmempoolinfo.minrelaytxfee` and Electrum `minrelaytxfee` follow `--min-relay-tx-fee`. `mempoolminfee`, Electrum `blockchain.relayfee`, and the BIP133 feefilter follow the live floor, including the near-full bump and its decay.
