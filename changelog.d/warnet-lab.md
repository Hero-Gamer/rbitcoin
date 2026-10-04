Fixed

- The Warnet kind image tag is `0.7.99-warnet0`, so Helm `semverCompare`
  renders the Bitcoin Core chart. The compose example stays
  `rbitcoin-warnet:local`.
- Lab tanks copy node log lines to stdout when `RBITCOIN_LOG_STDOUT=1`,
  and the RPC proxy accepts Warnet's fork-observer `rpcauth` login for
  its whitelisted chain methods while the tank `rpcpassword` user stays
  unrestricted. A tank whose conf omits `head_scale` uses tiny heads
  inside the lab image; `head_scale=mainnet` still selects mainnet heads.
