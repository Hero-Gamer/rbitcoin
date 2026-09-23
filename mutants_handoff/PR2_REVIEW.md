# PR2 Review - kill assemble_* mutants (consensus-critical)

Branch: test/kill-assemble-mutants-2
Base: bd449ab4 (upstream master, same as PR1)
Fork: Hero-Gamer/rbitcoin
Worktree: ~/rbitcoin-bip34 (vs ~/rbitcoin master)
Commit: current HEAD + this doc + mutants_handoff/

## Problem
docs/mutants/rbitcoin-consensus.md lists survivors in:
- assemble_lock_time_cutoff
- assemble_non_cb_tx
- assemble_tx_value_out

`cargo mutants -p rbitcoin-consensus --file crates/rbitcoin-consensus/src/block/mod.rs --re "assemble_lock_time_cutoff|assemble_non_cb_tx|assemble_tx_value_out" --timeout 30 -j2 -- --lib pr2_kills`

Found 34 mutants:
- 11 MISSED
- 2 TIMEOUT (> with < and >= in value_out - causes infinite loop)
- 16 caught, 5 unviable

### The 11 MISSED breakdown
```
MISSED 1188:27 > with == in assemble_non_cb_tx (block_sigops_cost > MAX)
MISSED 1196:12 < with > / <= in assemble_non_cb_tx (ti < ps.len() #1)
MISSED 1208:19 < with == / > / <= in assemble_non_cb_tx (ti < ps.len() #2 inside build_script_jobs)
MISSED 1223:5 -> Ok(0) in assemble_tx_value_out (whole function)
MISSED 1227:20 < with == in assemble_tx_value_out (sum < 0)
MISSED 1227:24 || with && in assemble_tx_value_out (sum<0 || sum>MAX)
MISSED 1227:31 > with >= in assemble_tx_value_out (sum > MAX)
MISSED 1236:29 > with == in assemble_tx_value_out (sats_u64 > MAX)
```

## What PR1 did (for reference)
PR1 killed bip34_height_script mutants (lines 65,68,69,70,73,76,79,82 in docs/mutants):
- 20 mutants, 13 caught, 1 timeout, 6 missed (dead neg branch from u32)
- Fix: added 32768 = [0x03,0x00,0x80,0x00] high-bit pad, 700k real mainnet height
- Waiver: neg branch unreachable because height:u32 => n:i64 >=0
- Branch: test/kill-bip34-height-mutants-3 @ fe3de0d1

## What PR2 is (current)
File: crates/rbitcoin-consensus/src/block/mod.rs lines 1180-1260
```rust
let tx_legacy_sigops = match pres.and_then(|p| p.get(ti)) {
    Some(p) => p.sigops.saturating_mul(4),
    None => legacy_sigop_count(tx).saturating_mul(4), // 20k*4=80k=MAX
};
*block_sigops_cost = checked_add...
if *block_sigops_cost > MAX_BLOCK_SIGOPS_COST { // 1188

if build_script_jobs {
    if ti < ps.len() { // 1208 - NEVER hit via validate_block_structure (build_script_jobs=false)
        job.with_pre_slice
    }
}

fn assemble_tx_value_out:
  const MAX_MONEY: i64 = 21M*100M
  if sum < 0 || sum > MAX { // 1227 - sum is p.out_sum as i64, out_sum: u64
  if sats_u64 > MAX as u64 { // 1236
```

File: crates/rbitcoin-consensus/src/block/structure_rule_tests.rs
Current pr2_kills tests (6):
- pr2_kills_zero_fee_and_zero_value (101 blocks maturity)
- pr2_kills_negative_output_and_div_zero
- pr2_kills_sigops_boundary: spend_ok 20k 0xac in script_sig+script_pubkey = 80k OK, 20_001 = 80_004 Err
- pr2_kills_lock_time_cutoff_genesis: genesis uses block.time, height1 uses MTP
- pr2_kills_value_out_max_money_boundary: 0 ok, 1 assert_eq!(1) kills Ok(0), MAX ok, MAX+1 err, sum_over err, pres_over via from_tx err kills ||->&&
- pr2_kills_ti_len_boundary: ti<len valid

TxPrecompute struct (11 fields, NO Default):
```rust
pub struct TxPrecompute { txid, wtxid, base_size, total_size, sigops, out_sum, has_witness, sha_prevouts, sha_sequences, sha_outputs, sha_amounts, sha_scriptpubkeys }
impl TxPrecompute { pub fn from_tx(tx: &Transaction) -> Self } // use this, not literal
```

## Why PR2 needs iteration (vs PR1 one-shot)
PR1: one function, one comparison, one boundary.

PR2: 3 checks stacked, with hidden branches:
1. `legacy_sigop_count` counts script_sig, not just script_pubkey - first fix only set output, counted 0
2. `ti < len` is inside `if build_script_jobs` which validate_block_structure sets false, so block tests never hit it - needs helper extraction
3. `sum < 0` needs u64::MAX as i64 = -1 to test negative path, and `|| with &&` needs MAX+1 only right side true
4. `-> Ok(0)` needs assert_eq! with specific value, not just is_ok()

Fixing one uncovers next branch was never executed. Normal for consensus code.

## Evidence (current)
cargo test -p rbitcoin-consensus --lib pr2_kills -- --nocapture => 6 passed

cargo mutants --re assemble_* => 11 missed (same as above)

## Fix Strategy to 0 missed (for next AI / Dola)
1. Extract helpers (allowed, doesn't change consensus):
```rust
#[inline] fn exceeds_sigops_limit(cost: u64) -> bool { cost > 80_000 }
#[inline] fn should_use_pres(ti: usize, len: usize) -> bool { ti < len }
#[inline] fn is_money_out_of_range(sum: i64) -> bool { sum < 0 || sum > MAX_MONEY }
```
Replace original ifs with helpers, then test helpers directly with boundaries: 79_999, 80_000, 80_001 and 0,1,MAX,MAX+1,-1 and (0,1) true, (1,1) false.

2. For Ok(0) mutant: keep assert_eq!(assemble_tx_value_out(tx with 1 sat), 1)

3. For pres over MAX: let mut pre = TxPrecompute::from_tx(&tx_over); pre.out_sum = MAX+1;

## What to check (Dola review)
- [ ] No consensus change: valid->invalid or invalid->valid?
- [ ] checked_add everywhere, not saturating_add for money?
- [ ] 20_000*4=80_000 exact boundary correct? Does legacy_sigop_count count OP_CHECKSIG in script_sig?
- [ ] ti < len helper extraction justified? Does it preserve build_script_jobs logic?
- [ ] MAX_MONEY = 2.1e15 < i64::MAX, cast safe? Check out_sum > i64::MAX before cast?
- [ ] Fee calc: checked_sub + fee<0 check preserved?
- [ ] Zero-fee tx still valid (regtest)?
- [ ] 101 blocks maturity still works?

## How to reproduce
cd ~/rbitcoin-bip34
cargo test -p rbitcoin-consensus --lib pr2_kills -- --nocapture
cargo mutants -p rbitcoin-consensus --file crates/rbitcoin-consensus/src/block/mod.rs --re "assemble_lock_time_cutoff|assemble_non_cb_tx|assemble_tx_value_out" --timeout 30 -j2 -- --lib pr2_kills
cat mutants_handoff/HANDOFF_PROMPT_FOR_NEXT_AI.md
cat mutants_handoff/ADVERSARIAL_REVIEW_CHECKLIST.md

## For next AI
Read mutants_handoff/HANDOFF_PROMPT_FOR_NEXT_AI.md first, then this file, then implement helper extraction.

## Push to fork
git add mutants_handoff/ PR2_REVIEW.md
git commit -m "doc: PR2 handoff for Dola - 11 missed assemble_* mutants, adversarial checklist"
git push origin test/kill-assemble-mutants-2
