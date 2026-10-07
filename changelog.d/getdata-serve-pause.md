Fixed

- **A `getdata` past the serve queue waits; it is not dropped.** When 16
  served blocks or 4 MiB were already queued to a peer, the rest of its
  `getdata` was dropped without a `block` or `notfound`. The peer waited
  until its stall timer, and an rbitcoin IBD peer then disconnected an
  honest node. As in Core, the session now keeps the rest of the request,
  sends the `notfound` it has, and serves every hash in order once the
  writer drains. No new message from that peer is read until then, but
  pings and the ping timeout still run. At most 16 served blocks sit in
  one peer's queue; a tip announce or `getblocktxn` block no longer frees
  a slot it never took.
