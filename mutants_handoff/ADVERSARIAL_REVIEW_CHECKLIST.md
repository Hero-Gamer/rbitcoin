# Adversarial Review Checklist - Bitcoin Consensus

## Before Any Fix
- [ ] Does this change ever make a previously valid block invalid? (chain split risk)
- [ ] Does this change ever make a previously invalid block valid? (inflation risk)
- [ ] Are you using `checked_add`/`checked_sub` everywhere money or sigops accumulates? Never `saturating_add` for money.
- [ ] Is MAX_MONEY = 21_000_000 * 100_000_000 = 2_100_000_000_000_000 sats = 2.1e15 < i64::MAX (9e18) so cast u64->i64 is safe?
- [ ] Is MAX_BLOCK_SIGOPS_COST = 80_000, and you multiply legacy_sigops by 4 (WITNESS_SCALE)?

## For Each Mutant Fix
- [ ] `> with ==` : Test exactly MAX (should pass) and MAX+1 (should fail). Both directions.
- [ ] `< with ==` : Test 0 (should pass) and -1 (should fail). For u64 out_sum, -1 comes from u64::MAX as i64.
- [ ] `< with <=` : Test ti == len (should NOT use pres) and ti == len-1 (should use pres)
- [ ] `|| with &&` : Test sum = MAX+1 (only right side true). Original `||` => Err, mutant `&&` => Ok (since left false). So Err test kills it.
- [ ] `-> Ok(0)` / `-> Ok(1)` : Test that function returns specific value, not just is_ok(). `assert_eq!(value_out(tx with 1 sat), 1)` kills Ok(0).

## Side Effects to Check
- [ ] Fee calculation: `fee = value_in - value_out` must use `checked_sub`, check `fee < 0`
- [ ] `value_out` accumulation: `checked_add` + check `> MAX_MONEY` after each add
- [ ] `block_sigops_cost`: `checked_add(tx_legacy_sigops)` + `checked_add(tx_in_sigops)` + check `> MAX`
- [ ] `pres` path: `p.out_sum as i64` - if out_sum > i64::MAX, cast wraps? Use check `out_sum > MAX as u64` BEFORE cast.
- [ ] No panics: `p.get(ti)` not `p[ti]` to avoid panic on ti==len, use `.get()`
- [ ] No dead code: `sats < 0` is impossible for u64, don't test it - test range instead

## Final Verification
- [ ] `cargo test -p rbitcoin-consensus --lib` - 397 tests pass
- [ ] `cargo mutants` with your filter - 0 missed
- [ ] `cargo clippy -p rbitcoin-consensus -- -D warnings` - no warnings
- [ ] Run with `RUST_BACKTRACE=1` on pr2_kills tests - no hidden panics
- [ ] Check that `mine_empty_regtest` still works for 101 blocks (maturity)
- [ ] Check zero-fee tx still valid (fee==0 is valid in regtest)

## What NOT to do
- Don't change `MAX_MONEY` value
- Don't change `MAX_BLOCK_SIGOPS_COST` value
- Don't use `saturating_add` for money (hides overflow)
- Don't add `#[allow(duplicate_macro_attributes)]` to silence double #[test] - fix the double #[test] instead
- Don't construct `TxPrecompute` with struct literal - use `from_tx()`

## Sign-off
If all checked, write in PR: "Adversarial review done: no chain split, no inflation, all boundaries tested, mutants 0 missed"
