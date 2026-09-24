# 051 — Replacement feerate is the direct conflict set

**Severity:** low
**Status:** won't-fix
**Found by:** @otaliptus (L-7)

Libre RBFR is documented as `new_rate ≥ 1.25 ×` the **direct** conflict
set's feerate, not each conflict individually and not the descendant
package. `pure_rbfr_pays` takes that direct fee and weight.
`rbf_allows_replacement` still also allows a BIP125-style payment against
the full conflict set. A high-feerate replacement with a lower absolute
fee than a fat descendant package is admitted by the direct-set rule.

**Regression:** `rbitcoin-mempool`
`accept::tests::pure_rbfr_unpins_descendant_package`,
`accept::tests::pure_rbfr_1_25x_ratio`.
