Fixed

- **Manual peers are not punished for misbehavior, as in Core.** A
  `--connect` or `addnode` peer that sends an invalid or mutated block, an
  oversized `inv`, `getdata` or `addrv2`, or a bad compact block now stays
  connected and its address is not refused. Core never disconnects or
  discourages a manual peer for misbehavior. Protocol violations that Core
  answers with a plain disconnect, such as `sendaddrv2` after `verack` or a
  `tx` to a `--blocks-only` node, still drop a manual peer.
- **`addnode` opens no second session to a connected address.** `addnode
  onetry` or `add` of an address with a live session, or one still being
  dialled, dialled it again, and each dial that connected became its own
  session. As in Core, the RPC now succeeds without dialling. A dial now
  counts from the moment it is queued, so an `addnode` or `--connect`
  redial right behind another dial to the same address adds nothing.
