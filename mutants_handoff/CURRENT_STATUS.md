# PR2 Mutants - Current Status (2026-05-13)

## Context
Project: rbitcoin-bip34 (reardencode/rbitcoin fork)
Package: rbitcoin-consensus
File: crates/rbitcoin-consensus/src/block/mod.rs
Functions under test:
- assemble_lock_time_cutoff
- assemble_non_cb_tx
- assemble_tx_value_out

## Last mutants run
```
cargo mutants -p rbitcoin-consensus \
  --file crates/rbitcoin-consensus/src/block/mod.rs \
  --re "assemble_lock_time_cutoff|assemble_non_cb_tx|assemble_tx_value_out" \
  --timeout 30 -j2 -- --lib pr2_kills
Found 34 mutants to test
ok       Unmutated baseline in 13s build + 8s test
MISSED   crates/rbitcoin-consensus/src/block/mod.rs:1188:27: replace > with == in assemble_non_cb_tx
MISSED   crates/rbitcoin-consensus/src/block/mod.rs:1196:12: replace < with > in assemble_non_cb_tx
MISSED   crates/rbitcoin-consensus/src/block/mod.rs:1196:12: replace < with <= in assemble_non_cb_tx
MISSED   crates/rbitcoin-consensus/src/block/mod.rs:1208:19: replace < with == in assemble_non_cb_tx
MISSED   crates/rbitcoin-consensus/src/block/mod.rs:1208:19: replace < with > in assemble_non_cb_tx
MISSED   crates/rbitcoin-consensus/src/block/mod.rs:1223:5: replace assemble_tx_value_out -> Result<i64, ConsensusError> with Ok(0)
MISSED   crates/rbitcoin-consensus/src/block/mod.rs:1208:19: replace < with <= in assemble_non_cb_tx
MISSED   crates/rbitcoin-consensus/src/block/mod.rs:1227:20: replace < with == in assemble_tx_value_out
MISSED   crates/rbitcoin-consensus/src/block/mod.rs:1227:24: replace || with && in assemble_tx_value_out
MISSED   crates/rbitcoin-consensus/src/block/mod.rs:1236:29: replace > with == in assemble_tx_value_out
MISSED   crates/rbitcoin-consensus/src/block/mod.rs:1227:31: replace > with >= in assemble_tx_value_out
TIMEOUT  crates/rbitcoin-consensus/src/block/mod.rs:1236:29: replace > with < in assemble_tx_value_out
TIMEOUT  crates/rbitcoin-consensus/src/block/mod.rs:1236:29: replace > with >= in assemble_tx_value_out
34 mutants tested: 11 missed, 16 caught, 5 unviable, 2 timeouts
```

## Evolution
- Run 1: 34 mutants, 17 missed (original code)
- Run 2: after adding MAX_MONEY checks, 11 missed
- Run 3: after fixing sigops to use script_sig + script_pubkey with 20k OP_CHECKSIG, still 11 missed
- Root causes identified:
  1. 1188 `> with ==` - cumulative sigops_cost, not per-tx. `legacy_sigop_count` counts script_sig, we were only setting script_pubkey before.
  2. 1196/1208 `ti < ps.len()` - inside `if build_script_jobs { }`. `validate_block_structure` has build_script_jobs=false, so branch never executed via block tests.
  3. 1223 `-> Ok(0)` whole function - need assert_eq!(value,1) not just is_ok()
  4. 1227 `|| with &&` and `< with ==`, `> with ==/>=` - pres path with out_sum = MAX+1 needed, built via TxPrecompute::from_tx()

## Current code at 1180-1260 (after last patch)
```
mod.rs not found
```

## Current pr2_kills tests (tail)
```
tests not found
```

## TxPrecompute struct
```
tx_precompute not found
```
