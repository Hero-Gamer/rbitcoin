Fixed

- The Warnet kind image tag is the release `28.0.0` (not an rbitcoin
  version). Helm `semverCompare ">=0.17.0"` is true for that tag, so the
  chart writes `[regtest]` before the tank's RPC and `addnode` lines.
  `0.7.99` and any prerelease of it compare below `0.17.0` and omit the
  section. The compose example stays `rbitcoin-warnet:local`.
- Lab tanks copy node log lines to stdout when `RBITCOIN_LOG_STDOUT=1`,
  and the RPC proxy accepts Warnet's fork-observer `rpcauth` login for
  its whitelisted chain methods while the tank `rpcpassword` user stays
  unrestricted. A tank whose conf omits `head_scale` uses tiny heads
  inside the lab image; `head_scale=mainnet` still selects mainnet heads.
