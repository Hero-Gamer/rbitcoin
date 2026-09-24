Changed

- Confirm stamps txstat size from the lookup precompute instead of walking
  each transaction again on the write thread.
- New `header.body` rows are 88 bytes. Opening a schema 24 or 25 store strips
  the old size/weight tail. Block size and weight are summed from txstat.
