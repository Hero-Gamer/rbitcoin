Fixed

- **Fee-flow buckets follow a log grid from min relay through 1000 sat/vB.**
  About 100 steps per decade. A 0.26 sat/vB inflow quotes its own step
  instead of 0.2, and rates above 1000 sat/vB share one open bucket. The
  inclusion search reads one suffix sum of those buckets.
