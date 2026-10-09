Changed

- **Fee-flow buckets are logarithmic from min relay through 1000 sat/vB.**
  About 100 steps per decade, so a 0.26 sat/vB inflow quotes its own step
  instead of 0.2. Rates above 1000 sat/vB share one open bucket.
