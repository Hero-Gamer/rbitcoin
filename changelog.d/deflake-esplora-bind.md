Fixed

- **A taken Esplora port no longer fails the cross-surface journey.** The journey binds `127.0.0.1:0` and reads the address the node publishes under `{datadir}/run/*.addr`. A failed Electrum or Esplora bind is not retried on the tip loop.
