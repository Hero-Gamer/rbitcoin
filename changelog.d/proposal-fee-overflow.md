Fixed

- **A block proposal whose fees do not fit in `u64` is rejected.** Output
  totals, input totals, and the fee sum use checked addition. An overflowing
  sum is `bad-txns-txouttotal-toolarge`, `bad-txns-inputvalues-outofrange`,
  or `bad-txns-fee-outofrange` instead of a successful template check.
