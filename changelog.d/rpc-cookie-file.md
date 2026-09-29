Added

- **Core cookie auth on TCP RPC:** `--rpc-cookie-file PATH` (conf
  `rpc_cookie_file=`, NixOS `services.rbitcoin.rpc.cookieFile`) accepts an
  existing Core-format `username:password` file as HTTP Basic on the TCP
  listener, alongside the Bearer token, so stock mempool `CORE_RPC.COOKIE`
  authenticates without a patch; the unix-socket `socketPath` patch is now
  optional. The node never creates the file. It must have no trailing
  newline (mempool sends the raw bytes, so one would 401 forever) and needs
  `--rpc-listen`; either mistake fails the launch. The TCP 401 challenge is
  `Basic` when a cookie is configured, `Bearer` otherwise.
- **Core-shaped confirmed transaction JSON:** confirmed verbose
  `getrawtransaction` adds `confirmations`, `blockhash`, `blocktime`, and
  `time`. This also fixes the coinbase input in `getrawtransaction` /
  `decoderawtransaction` / `getblock` verbosity 2: it is now
  `{"coinbase": <hex>, "sequence": n}` (plus `n`) instead of a
  `txid`/`vout` pair that never existed. mempool needs both to index
  blocks.
