Removed

- **Esplora WebSocket (`/ws`, `/v1/ws`).** It copied mempool.space's
  `/api/v1/ws`, which is mempool's backend surface; Esplora and electrs
  have no WebSocket. Both paths now 404. Watch wallets over Electrum
  subscriptions, or run mempool's backend in front of this Esplora.
  `EsploraConfig` drops the `max_ws_*` / `max_track_*` caps and
  `run_esplora` drops its tip-broadcast argument.
