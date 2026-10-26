Fixed

- **A scripthash or index write-behind IO error exits non-zero.** It used
  the same shutdown as SIGTERM, so `Restart=on-failure` left the unit down.
  SIGTERM, Ctrl+C, and RPC `stop` still exit 0. A tip-connect store fault
  still keeps the session and does not exit; it releases the body ask so
  the block can be fetched again.
