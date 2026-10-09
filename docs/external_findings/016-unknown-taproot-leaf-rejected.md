# Unknown Taproot leaf versions are rejected (over-strict)

**Component:** `rbitcoin-consensus` (`script/p2tr.rs::verify_script_path`)
**Audit pin:** fuzzamoto report 005 / rbitcoin `8f3990f`
**Severity:** critical — stalls on a block Core accepts (upgrade path)
**Status:** fixed — `script_path_accepts_leaf_0x50_with_annex_and_0xc2` (leaf `0xc2` succeeds)
**Found by:** fuzzamoto

## Summary

After BIP341 commitment verification, a leaf version other than tapscript
(`0xc0`) returns `Script("p2tr leaf version")`. BIP341 requires unknown leaves
to **succeed** without executing the leaf so future soft forks stay compatible.
Core only **discourages** them in mempool policy flags, not block validation.

## Fix

Commitment OK + leaf ≠ TapScript → `Ok(())`. Execute only tapscript. Do not
apply discourage flags on the block path.
