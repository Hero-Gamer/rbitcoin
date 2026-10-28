Changed

- **Electrum `asof:` needs no dialect.** A trailing `asof:<blockhash>` on
  `get_history`, `get_balance`, `listunspent`, `transaction.get` and
  `get_merkle` answers as of that block on any negotiated protocol. It was
  refused unless the connection had negotiated `1.4.2-asof`, which also held
  that connection to 1.4.x shapes, so a 1.6 client could not use as-of. Clients
  that negotiate `1.4.2-asof` keep working unchanged.
