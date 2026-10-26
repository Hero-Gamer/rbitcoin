Fixed

- **A scripthash or index write-behind IO error exits non-zero.** It used
  the same shutdown as SIGTERM, so `Restart=on-failure` left the unit down.
  SIGTERM, Ctrl+C, and RPC `stop` still exit 0.
