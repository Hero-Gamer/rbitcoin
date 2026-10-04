Changed

- **Mempool expiry scans the oldest accepts first.** Each pass still visits
  at most 256 live transactions. The headers poll skips the pass when that
  index is busy, and still runs off the async worker.
