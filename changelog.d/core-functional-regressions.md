Fixed

- **Misbehavior no longer disconnects `noban` peers.** A peer with the
  `noban` permission (`--trusted`, or `noban` in `--net-permission` /
  `--net-permission-bind`) that sends an invalid block or header is
  logged and kept, as in Core.
- **Rejected block headers log Core's reason.** A block whose header fails
  contextual checks logs `bad-version(0x…)` or `time-too-new` again.
- **Relayed addresses go out in one `addrv2` per peer.** Previously each
  address was sent as its own message.
- **Parked RPC waits show in `getrpcinfo`.** `waitfor*` and a
  `getblocktemplate` longpoll are listed in `active_commands` while they
  wait, and a longpoll logs its request when it arrives.
