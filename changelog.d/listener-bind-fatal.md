Changed

- **A listener that cannot bind stops the node.** A busy `--rpc-listen`,
  `--electrum-listen`, `--esplora-listen`, or `--sv2-tp-listen` port, or an
  `--rpc-socket` directory that cannot be created, used to log a warning
  while the node kept running without it. A busy RPC port also took the
  unix socket down. The node now exits 1 with
  `configuration error: <name> start: …`, as `--health-listen` and
  `--listen` already did. `/readyz` no longer has a `not listening` reason.
