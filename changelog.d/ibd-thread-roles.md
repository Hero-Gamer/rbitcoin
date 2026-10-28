Changed

- The IBD header thread is named `ibd-header` for the catch-up, then joined.
- `rbtc-sh-wb` starts when `--sh-index` has the index in Tip. A durable head starts it before catch-up. Direct catch-up starts it only after materialize. It stays down when materialize is cancelled or fails.
- IBD Class C barriers share one `rbtc-tip-flush` thread for the confirm engine. Tip mode still flushes that barrier on the tip-accept thread.
- Heavy pure CPU leaves the blocking pool for `rbtc-scripts-*`: v2 encode when `encode_is_cpu_heavy`, compact short ids after the body is in hand, and Electrum or Esplora JSON of at least 4 KiB, including `GET /block-template`. Shorter encodes stay on the caller. Block templates still send `cache-control: no-store`. Block and header decode stay on the blocking pool.
- A panic inside one of those script-pool jobs does not kill the worker. A dropped JSON encode is an error, not `{}`. A dropped compact-block encode is a peer protocol error, not a header fallback.
