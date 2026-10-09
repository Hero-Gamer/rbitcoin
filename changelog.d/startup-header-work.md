Fixed

- **A restart totals header work with two sequential reads of `header.body`.** One read ranks a stored side chain against the tip. The next fills chain work through the tip, from `nBits` alone. Catch-up finishes without a separate read of every header row. A header that arrives while that read is starting does not abort the search. A failed chain-work total does not keep a partial sum.
