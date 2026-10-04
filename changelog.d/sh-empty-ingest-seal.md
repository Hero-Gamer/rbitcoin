Fixed

- **`--sh-index` on an empty chain seals scripthash shards without reading
  the empty ingest table.** That walk is 2^25 slots per shard and was
  holding tip entry past the RPC cookie window.
