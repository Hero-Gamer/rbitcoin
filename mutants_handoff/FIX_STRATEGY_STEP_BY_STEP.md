# Fix Strategy for Remaining 11 Missed

## Current missed (11 + 2 timeout)
```
1188:27 > with == in assemble_non_cb_tx (sigops cumulative)
1196:12 < with > and <= in assemble_non_cb_tx (pres len check #1)
1208:19 < with == / > / <= in assemble_non_cb_tx (pres len check #2 - inside build_script_jobs)
1223:5 -> Ok(0) in assemble_tx_value_out (whole function)
1227:20 < with == in assemble_tx_value_out (sum < 0)
1227:24 || with && in assemble_tx_value_out (sum <0 || sum>MAX)
1227:31 > with >= in assemble_tx_value_out (sum > MAX)
1236:29 > with == / < / >= in assemble_tx_value_out (sats_u64 > MAX)
```

## Step-by-step to 0 missed

### Step 1: Kill sigops 1188
Current test at 2230-2245 uses:
```
spend_ok.input[0].script_sig = vec![0xac; 20_000]
spend_ok.output[0].script_pubkey = vec![0xac; 20_000]
```
20_000 *4 = 80_000 = exactly MAX. Should be OK.
20_001 *4 = 80_004 = over MAX. Should be Err.

But `validate_block_structure` may have `build_script_jobs=false`, so sigops still counted? Yes, sigops counted before that if.

Check: Does `non_coinbase_spend(0)` have value 0? Need mature coinbase. Use existing helper `non_coinbase_spend` which already spends mature.

If still missed, extract helper:
```rust
fn exceeds_sigops_limit(cost: u64) -> bool { cost > 80_000 }
```
Test helper directly with 79_999, 80_000, 80_001.

### Step 2: Kill ti < len 1196/1208
These are inside:
```rust
if build_script_jobs {
    if ti < ps.len() { job.with_pre_slice }
}
```
`validate_block_structure` does NOT set build_script_jobs=true, so never hits.
Solution: Extract helper `should_use_pres(ti,len) -> bool { ti < len }` and test helper:
```
assert!(should_use_pres(0,1))
assert!(!should_use_pres(1,1))
assert!(!should_use_pres(0,0))
```
Then replace original `if ti < ps.len()` with `if should_use_pres(ti, ps.len())`. Now mutants are in helper and killed.

### Step 3: Kill Ok(0) and money range
Current test `pr2_kills_value_out_max_money_boundary` has:
- tx0 0 sat -> is_ok
- tx1 1 sat -> assert_eq!(1) -> kills Ok(0) because Ok(0) would return 0 not 1
- tx_max MAX -> is_ok
- tx_over MAX+1 -> is_err -> kills > mutants
- tx_sum_over MAX + 1 -> is_err
- pres_over via from_tx -> is_err -> kills || with &&

If still missed, check: `from_tx` for tx_over with MAX+1 may itself fail because Amount::from_sat(MAX+1) is valid but `from_tx` may cap out_sum? Print out_sum.

Better: Build pres manually with `out_sum = MAX+1` using from_tx then override out_sum field? But TxPrecompute has no Default. Use from_tx then mutate via `let mut pre = TxPrecompute::from_tx(&tx); pre.out_sum = MAX+1;`

But out_sum is pub, so you can mutate.

### Step 4: Run final verification
```
cargo test -p rbitcoin-consensus --lib pr2_kills -- --nocapture
cargo mutants -p rbitcoin-consensus --file ... --re "exceeds_sigops_limit|should_use_pres|is_money_out_of_range|assemble_tx_value_out|assemble_non_cb_tx" -- --lib pr2_kills
cargo test -p rbitcoin-consensus --lib
```

Goal: 0 missed.

## Why iteration needed
Each fix uncovers that previous test didn't hit branch (build_script_jobs false, script_sig vs script_pubkey, pres vs no pres). So you need to extract helpers to make branch testable.
