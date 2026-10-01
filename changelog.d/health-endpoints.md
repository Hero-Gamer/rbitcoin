Added

- **Health probes.** `--health-listen [ADDR]` (default `127.0.0.1:9332`)
  binds before the store opens and serves `GET /healthz` (200 in every
  phase) and `GET /readyz` (200 once the node follows the tip with every
  configured listener up, the tip within 6 blocks of the best header, the
  tip fresher than `--max-tip-age`, and the scripthash index within 6
  blocks of the tip; otherwise 503 with the reason). The stale-tip check
  does not latch like `initialblockdownload`, so a node that loses every
  peer after IBD goes unready. RPC, Electrum, and Esplora bind only after
  catch-up, so a probe on those would restart a node in the middle of a
  migration or IBD.
- **Prometheus metrics.** `--metrics` adds `GET /metrics` on the health
  listener. Gauges equal their RPC fields (`blocks`, `headers`,
  `initialblockdownload`, connections, mempool size). Counters are the
  `tip: perf` meters (Esplora and Electrum requests, historical block
  serves, mempool accepts and rejects), which now count up for the life
  of the process; the 5 s DEBUG line still prints the change since the
  previous line.
