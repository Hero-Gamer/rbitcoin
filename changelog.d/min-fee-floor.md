Fixed

- **A replacement that pays the incremental minimum is no longer rejected for sharing a truncated sat/kvB bucket.** RBF requires a strictly higher true feerate, including when `fee × vsize` exceeds 2^64.
- **Advertised minimums match the fee the mempool enforces.** `getmempoolinfo.minrelaytxfee` and Electrum `minrelaytxfee` follow `--min-relay-tx-fee`. `mempoolminfee`, Electrum `blockchain.relayfee`, and the BIP133 feefilter follow the live floor, including the near-full bump and its decay.
- **An admission that evicts other transactions and then fails still publishes that higher floor.** `mempoolminfee`, Electrum `blockchain.relayfee`, and the feefilter update immediately, including when the failed transaction was the first member of a package.
