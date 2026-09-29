Fixed

- **Block sigop cap includes 80,000.** Admission and template selection
  allow a running cost of exactly 80,000, with `--block-reserved-sigops`
  counted in that total. A cost that would pass 80,000 is still
  `bad-txns-too-many-sigops`.
