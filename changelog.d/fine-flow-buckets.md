Fixed

- **Fee-flow buckets match the 0.1 sat/vB quote grid.**
  Admit rates through 10 sat/vB are kept in 0.1 sat/vB steps, then at 20,
  50, and 100 sat/vB. A 1.5 sat/vB inflow no longer shares a bucket with
  1.0, and a 4.9 sat/vB inflow no longer quotes 2.0. The inclusion search
  reads one suffix sum of those buckets.
