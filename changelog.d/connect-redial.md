Fixed

- **A `--connect` address is redialled when its session drops.** Only
  `--connect` hostnames were retried. An IP, onion, I2P or CJDNS target got
  one dial at startup, so a node whose `--connect` peer restarted or was
  disconnected stayed without peers until it was restarted. As in Core,
  once the node follows the tip, every `--connect` target without a live
  session is redialled every 2 s, including any that the startup dial
  skipped (it dials at most three).
- **A redial does not stack while a dial is still connecting.** The 2 s
  redial of `--connect` and `addnode add` targets skips a target whose
  previous dial has not finished connecting. Before, a target behind a slow
  SOCKS circuit or a host that drops SYNs got a new dial every pass, and
  each one that connected became its own session.
