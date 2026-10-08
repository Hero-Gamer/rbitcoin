Fixed

- **Pure replace-by-fee-rate compares the full product.** A replacement whose
  `new_fee * 4 * old_vsize` or `direct_fee * 5 * new_vsize` exceeds `u64`
  is held to 1.25× in `u128`, instead of a saturating multiply that can
  accept a rate below the rule.
