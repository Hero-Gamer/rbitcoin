Changed

- **Unknown version bits follow BIP 323.** Only bits 0–4 can raise
  "Unknown new rules activated". Higher bits stay quiet.
- **A block-relay-only peer's transaction `getdata` is ignored.** The node
  does not send the transaction and does not answer `notfound`.
- **A `sendcmpct` announce byte other than 0 or 1 disconnects that peer.**
- **`-blocksonly` ignores an incoming compact block,** including one it
  asked for as a full block. It still serves compact blocks.
- **`getblocktxn` is sent only to a high-bandwidth peer,** or for a block
  this node already asked for, and only after that peer sent `sendcmpct`
  version 2. A compact block whose transactions are already in the message
  is still checked. Bad proof of work still disconnects.
- **A filtered-block `getdata` disconnects the peer.** `NODE_BLOOM` is off.
- **`--external-ip` accepts a v3 onion.** It is listed in
  `getnetworkinfo` local addresses. That does not make onion reachable.
- **`estimatesmartfee` takes an `options` object.** `fee_rate_estimator`
  and `verbosity` are checked. The feerate stays the 10-minute product.
- **`echojson` is the same probe as `echo`.**
