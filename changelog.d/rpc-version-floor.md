Changed

- **`getnetworkinfo.version` is `190000`.** That is Bitcoin Core 0.19.0's
  client integer, so typed RPC clients take the modern response path.
  `bitcoincore-rpc` `get_blockchain_info` otherwise requires the
  pre-0.19 `bip9_softforks` map and fails on our object. The rbitcoin
  semver stays in `subversion`. `protocolversion` stays `70016`.
