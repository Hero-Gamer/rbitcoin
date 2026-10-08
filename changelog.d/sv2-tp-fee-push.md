Added

- **SV2 TP fee-gain templates.** With the tip unchanged, a session gets a
  new template once its fees gain `--sv2-tp-fee-delta` sats (default
  1000) over the last one sent, checked every
  `--sv2-tp-template-interval` seconds (default 5). NixOS:
  `services.rbitcoin.sv2.tp.{feeDelta,templateInterval}`.
