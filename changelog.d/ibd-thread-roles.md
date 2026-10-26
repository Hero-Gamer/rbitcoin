Changed

- The IBD header thread is named `ibd-header` for the catch-up, then joined.
- `rbtc-sh-wb` starts after tip entry, and only when `--sh-index` is on and the index is in Tip. It stays down for Direct catch-up, including a cancelled or failed scripthash materialize.
- IBD Class C barriers share one `rbtc-tip-flush` thread for the confirm engine. Tip mode still flushes that barrier on the tip-accept thread.
