Fixed

- **NixOS index options no longer crash-loop the node.** `scripthashIndex`,
  Electrum, and Esplora pass `--sh-index`. `silentPaymentIndex` passes
  `--sp-tweaks`. The old spellings were rejected at startup, and systemd
  restarted the unit every 10 seconds.
