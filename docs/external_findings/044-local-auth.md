# 044 — Local auth and datadir mode

**Severity:** low
**Status:** fixed
**Found by:** otaliptus review (L-1, L-3, L-12, loopback note)

Tor control cookie auth is SAFECOOKIE only. A control port that does not
advertise SAFECOOKIE, or a cookie that is not 32 bytes, does not send
`AUTHENTICATE` with the raw cookie hex. Password auth is unchanged.
A short control reply was already rejected before the 3-byte status slice.
I2P SAM does not use that slice.

A datadir this process creates is mode `0700`, including under umask
`0022`. An existing directory is not chmodded.

`rpc.sock` is created mode `0600` and the Esplora unix socket mode `0660`
by the umask around bind, before accept.

A `--net-permission` grant for `127.0.0.1` does not apply to a peer whose
address is onion or I2P. Inbound Tor and I2P still arrive as the loopback
TCP peer on the shared P2P socket, so that socket address is what the
grant sees.

**Regression:** `rbitcoin-node` `tor_control::tests::tor_plain_cookie_is_not_sent_when_safecookie_is_absent`,
`tor_short_cookie_does_not_fall_back_to_raw_hex`,
`config::tests::builders_paths_milestone_and_ensure`,
`rbitcoin-rpc` `server::tests::unix_socket_needs_no_http_auth`,
`rbitcoin-esplora` `server::tests::unix_listen_serves_tip_height`,
`rbitcoin-net` `net_permissions::tests::loopback_grant_does_not_cover_onion_or_i2p`.
