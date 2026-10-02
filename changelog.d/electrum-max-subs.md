Changed

- **Electrum subscription cap is 10000 per connection and configurable**
  (`--electrum-max-subs N`, conf `electrum_max_subs=`, NixOS
  `services.rbitcoin.electrum.maxSubs`). The old fixed 1000 refused an
  ordinary Sparrow wallet mid-sync (`too many scripthash subscriptions`),
  because a wallet subscribes every receive and change address up to its gap
  limit. Per-event costs that grew with the subscription count are gone: a
  mempool accept probes the subscription set with the tx's scripthashes; a new
  block builds its touch set once (`Query::block_touch`) instead of reloading
  the block and its prevouts per subscribed hash; requests move the sub sets
  instead of cloning them; and unsubscribe drops its last-sent status exactly.
  A reorg (or a tick gap > 32) still restatuses every subscription in full.
