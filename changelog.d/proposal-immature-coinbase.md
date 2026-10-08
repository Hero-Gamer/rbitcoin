Fixed

- **`getblocktemplate` proposal mode rejects an immature coinbase spend.**
  Spending this block's coinbase, or a coinbase still inside the maturity
  window, is `bad-txns-premature-spend-of-coinbase` before fees are summed.
  That spend no longer inflates the fee total returned to a template
  provider, and it no longer hides `bad-cb-amount`. Structure checks still
  run first.
