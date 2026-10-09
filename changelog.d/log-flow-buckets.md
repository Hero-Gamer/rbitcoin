Changed

- **Fee-flow buckets are logarithmic from min relay through 1000 sat/vB.**
  About 100 steps per decade, so a 0.26 sat/vB inflow quotes its own step
  instead of 0.2. Rates above 1000 sat/vB share one open bucket.
- **Flow fullness blends history into the fee quote.** The estimate starts
  as history and becomes the live flow quote as decayed admitted weight
  reaches about 1.44M WU. A quiet stretch falls back toward history.
