Fixed

- **Scripthash index survives power loss.** Write-behind now syncs the
  scripthash tables before advancing the durable inclusion watermark.
  Before, an OS crash or power cut could leave the watermark claiming
  outputs whose index bytes were lost, so those addresses' histories
  stayed incomplete. `tip: accept` shows the sync time as `sync=`.
- **Silent-payment tweak index survives crashes.** Each write is synced,
  and on start the index is trimmed to the chain tip and its last record
  checked. Before, a crash during a reorg could keep serving tweaks for
  blocks no longer on the chain, and a power cut could turn tweaks into
  "none" or make a height unreadable.
