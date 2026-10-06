Fixed

- **Signet `OP_TRUE` can be mined on demand.** `generatetoaddress` /
  `generate` / `generateblock` work on regtest and on a signet whose
  challenge is exactly `OP_TRUE` (signet bits, empty BIP325 solution).
  `getblockchaininfo.signet_challenge` is present on signet. A signet
  solution failure is reported as `bad-signet-blksig`.
- **`getblockfilter` serves a stored block that is not sealed yet.** A
  best-chain height already in the index still uses that row. A known
  block that is stale or not sealed is rebuilt from the body. An unknown
  `filtertype` is `-5`. REST `/rest/blockfilter/` stays watermark-only.
- **`submitpackage` package-evaluates a child-with-parents remainder.**
  Members that failed static min relay, the dynamic mempool floor, or
  missing inputs are retried together, including when the child spends a
  parent already in the mempool. The dynamic floor is waived when the
  remainder meets static min relay. Effective feerate for that remainder
  is the summed modified fees over the summed vsizes. Trim runs once
  after the package; a tx that was admitted and then dropped is
  `mempool full`. Package-admitted txs use the same relay-age clock as
  an individual admit, and `prioritisetransaction` deltas count in the
  feerate announced to peers. A member whose own feerate is above
  `maxfeerate` is rejected and does not rescue the rest of the package.
  An individual admit still reports the transactions it replaced.
- **`getmempoolinfo.maxmempool` is the virtual-size byte cap.** The hub
  budget stays weight (`--mempool-size-mb` × 1_000_000). The Core field is
  that budget divided by 4, the same unit as `bytes`.
  `rbitcoin_mempool_max_weight` still reports weight.
- **An unspendable output is still a coin.** A script that starts with
  `OP_RETURN`, or is longer than 10_000 bytes, resolves and fails in the
  script. Core functional `generateblock` text for that spend is shim-only.
