# PROMPT FOR NEXT AI - PR2 Consensus Mutants - Air-Tight Fix Required

You are taking over a critical Bitcoin consensus fix. This is NOT a normal bug fix. Any mistake = chain split / loss of funds. You must be adversarial, paranoid, and air-tight.

## Your Mission
Kill ALL 34 mutants in:
```
crates/rbitcoin-consensus/src/block/mod.rs
--re "assemble_lock_time_cutoff|assemble_non_cb_tx|assemble_tx_value_out"
```
Current: 11 MISSED, 2 TIMEOUT. Goal: 0 MISSED.

Command:
```
cargo mutants -p rbitcoin-consensus   --file crates/rbitcoin-consensus/src/block/mod.rs   --re "assemble_lock_time_cutoff|assemble_non_cb_tx|assemble_tx_value_out"   --timeout 30 -j2 -- --lib pr2_kills
```

## Context You Must Understand

### Why PR2 is harder than PR1 (user's question)
PR1 was structure rules: `if height==0 { block.time } else { MTP }` - one comparison, one mutant.
PR2 is money + sigops + cache in ONE function `assemble_non_cb_tx`:

```rust
// 1180-1250 current code (after patches)
let tx_legacy_sigops = match pres.and_then(|p| p.get(ti)) {
    Some(p) => p.sigops.saturating_mul(4),
    None => legacy_sigop_count(tx).saturating_mul(4), // 20k *4 = 80k = MAX
};
*block_sigops_cost = block_sigops_cost
    .checked_add(tx_legacy_sigops)
    .and_then(|c| c.checked_add(tx_in_sigops))
    .ok_or(BadBlock("bad-blk-sigops"))?;
if *block_sigops_cost > MAX_BLOCK_SIGOPS_COST { // 1188 MISSED > with ==
    return Err(BadBlock("bad-blk-sigops"));
}
let value_out = assemble_tx_value_out(tx, ti, pres)?;

fn assemble_tx_value_out(...) -> Result<i64> {
    const MAX_MONEY: i64 = 21_000_000 * 100_000_000;
    match pres.and_then(|p| p.get(ti)) {
        Some(p) => {
            let sum = p.out_sum as i64;
            if sum < 0 || sum > MAX_MONEY { // 1227 MISSED < with ==, || with &&, > with >=
                return Err(BadTx("value out of range"));
            }
            Ok(sum)
        },
        None => {
            let mut value_out = 0i64;
            for o in &tx.output {
                let sats_u64 = o.value.to_sat();
                if sats_u64 > MAX_MONEY as u64 { // 1236 MISSED > with == / < / >=
                    return Err(BadTx("output value too large"));
                }
                value_out = checked_add...
                if value_out > MAX_MONEY { // 1244
                    return Err(BadTx("tx output sum too large"));
                }
            }
            Ok(value_out)
        }
    }
}
```

### The 11 Missed Mutants Breakdown

1. **1188:27 replace > with == in assemble_non_cb_tx**
   - `if *block_sigops_cost > 80_000`
   - Needs block with exactly 80_000 sigops = OK, 80_001 = Err
   - Trap: `legacy_sigop_count` counts `script_sig`, not just `script_pubkey`. Previous test only set output.
   - Trap: `block_sigops_cost` is cumulative. 20_000 *4 = 80k. Must use `non_coinbase_spend` not coinbase.
   - Fix: Put OP_CHECKSIG (0xac) in BOTH input and output, 20_000 for OK, 20_001 for over.

2. **1196:12 and 1208:19 replace < with > / <= / == in assemble_non_cb_tx**
   - `if ti < ps.len()` where `ps = pres: Option<&Arc<[TxPrecompute]>>`
   - This is inside `if build_script_jobs { if ti < ps.len() { job.with_pre_slice } }`
   - `validate_block_structure` sets `build_script_jobs=false`, so this branch is NEVER executed via block tests.
   - To kill: you MUST call `assemble_non_cb_tx` directly with `build_script_jobs=true`, or extract helper `fn should_use_pres(ti,len)->bool { ti < len }` and test helper directly. Helper extraction is allowed and makes mutants easier to kill.

3. **1223:5 replace assemble_tx_value_out -> Result with Ok(0)**
   - Whole function replaced with Ok(0). Any test that only checks `is_ok()` still passes.
   - Must have `assert_eq!(assemble_tx_value_out(tx1,0,None).unwrap(), 1)` where tx1 has 1 sat output. Ok(0) would return 0, failing.

4. **1227:20 < with ==, 1227:24 || with &&, 1227:31 > with >=, 1236:29 > with ==**
   - `if sum < 0 || sum > MAX_MONEY`
   - `sum` is `p.out_sum as i64` where `p.out_sum: u64`. To get sum <0, need out_sum = u64::MAX (as i64 = -1) or out_sum > i64::MAX.
   - `|| with &&` : `sum<0 && sum>MAX` is impossible, so mutant never errors. Need test with pres where out_sum = MAX+1, expecting Err. If mutant returns Ok(MAX+1), test fails -> caught.
   - Build pres via `TxPrecompute::from_tx(&tx_over)` where tx_over has output MAX+1, NOT via struct literal (struct has 11 fields, no Default).
   - Example:
     ```rust
     let tx_over = Transaction { output: vec![TxOut { value: Amount::from_sat(MAX+1), .. }] };
     let pre = TxPrecompute::from_tx(&tx_over);
     let pres: Arc<[TxPrecompute]> = Arc::from([pre]);
     assert!(assemble_tx_value_out(&tx0, 0, Some(&pres)).is_err());
     ```

### TxPrecompute struct (no Default)
```rust
pub struct TxPrecompute {
    pub txid: [u8; 32],
    pub wtxid: [u8; 32],
    pub base_size: usize,
    pub total_size: usize,
    pub sigops: u64,
    pub out_sum: u64,
    pub has_witness: bool,
    pub sha_prevouts: Option<[u8; 32]>,
    pub sha_sequences: Option<[u8; 32]>,
    pub sha_outputs: Option<[u8; 32]>,
    pub sha_amounts: Option<[u8; 32]>,
    pub sha_scriptpubkeys: Option<[u8; 32]>,
}
impl TxPrecompute { pub fn from_tx(tx: &Transaction) -> Self { ... } }
```
Use `from_tx`, never manual literal.

## Your Deliverables (Air-Tight)

1. **No consensus change**: Your fix must NOT change valid blocks to invalid or invalid to valid. Only add checks or make existing checks more explicit. Use `checked_add`, not `saturating_add` for money.
2. **Boundary tests for EVERY mutant operator**:
   - `>` needs tests at MAX, MAX+1, MAX-1
   - `<` needs 0, -1 (via u64::MAX), 1
   - `||` needs both sides false, one true, both true (impossible case)
   - Whole function Ok(0)/Ok(1) needs assert_eq! with specific values
3. **Adversarial review**: After fixing, run:
   - `cargo test -p rbitcoin-consensus --lib pr2_kills -- --nocapture`
   - `cargo mutants ...` -> must be 0 missed
   - `cargo test -p rbitcoin-consensus --lib` (all 397 tests)
   - Check for negative side effects: fee overflow, sigops overflow, future money supply, time warp
4. **Document why each mutant is killed**: Comment in test: `// kills > -> == : exactly MAX must be ok`

## Files to Request from User
- crates/rbitcoin-consensus/src/block/mod.rs (full, lines 1140-1260 critical)
- crates/rbitcoin-consensus/src/block/structure_rule_tests.rs (all pr2_kills_*)
- crates/rbitcoin-query/src/tx_precompute.rs
- Output of `cargo mutants ...` (current 11 missed)

## Success Criteria
- 0 missed, 0 timeout in the 34 mutants set
- All 397 existing tests still pass
- No new clippy warnings, no `unwrap` in consensus path (use `checked_add` + `ok_or`)
- Tests include comments linking to mutant operator killed

## Example of Air-Tight Helper Extraction (Allowed)
Instead of directly testing private complex function, extract:

```rust
#[inline]
fn exceeds_sigops_limit(cost: u64) -> bool {
    const MAX: u64 = 80_000;
    cost > MAX
}
#[inline]
fn should_use_pres(ti: usize, len: usize) -> bool { ti < len }
#[inline]
fn is_money_out_of_range(sum: i64) -> bool { sum < 0 || sum > MAX_MONEY }
```

Then test helpers directly with boundaries. This makes mutants in helpers trivial to kill and does NOT change consensus (same logic).

Good luck. Be paranoid.
