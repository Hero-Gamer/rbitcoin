Changed

- **Esplora fee answers keep the estimator's precision.** `/fee-estimates`
  serves sat/vB to 0.001 (whole sat/kvB) instead of rounding to 0.1.
- **Esplora no longer serves `/fees/recommended` or
  `/v1/fees/recommended` (404).** Those are mempool.space's backend API
  (`/api/v1/`), not Esplora's. Use `/fee-estimates`, or mempool's backend
  in front of rbitcoin.
- **Esplora no longer invents a 1 sat/vB fee.** `/fee-estimates` leaves
  out targets the estimator cannot answer and returns **503** `fee
  estimates unavailable` when none has an answer. Electrum `blockchain.estimatefee` keeps the protocol's `-1`.
