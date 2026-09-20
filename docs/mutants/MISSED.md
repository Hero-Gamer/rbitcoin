# Cargo Mutants Baseline

Generated: `cargo mutants -p rbitcoin-primitives -j 2` and `cargo mutants -p rbitcoin-consensus -j 2`
Date: 2026-05-13
Runner: NeoAIs-Mac-mini-4 /tmp/rbtc-628-fix-628

## Summary

| Crate | Tested | Missed | Caught | Unviable | Timeout |
| :--- | ---: | ---: | ---: | ---: | ---: |
| rbitcoin-primitives | 292 | 15 | 194 | 79 | 4 |
| rbitcoin-consensus | 2518 | 304 | 1383 | 793 | 38 |

After sigop fix in this branch (2d15e3a): primitives -> 12 missed, script_sigops.rs -> 2 missed (equiv at 41:39)

## rbitcoin-primitives - 15 missed

- lib.rs:62:16 `> with >= in rbitcoin_subversion`
- hex.rs:96:28 `| with ^ in decode`
- hex.rs:96:28 `| with & in decode`
- loc_simd.rs:11:18 `== with != in deinterleave_pairs_u8x8_scalar`
- loc_simd.rs:11:23 `|| with && in deinterleave_pairs_u8x8_scalar`
- script_sigops.rs:21:15 `+= with -=` - KILLED by [4c,02,ac] in this PR
- script_sigops.rs:21:15 `+= with *=` - KILLED by [4c,01,ac] in this PR
- script_sigops.rs:28:15 `+= with *=` - KILLED by [4d,01,00,ac] in this PR
- script_sigops.rs:28:15 `+= with -=` - KILLED
- script_sigops.rs:41:39 `> with ==` - EQUIV
- script_sigops.rs:41:39 `> with <` - KILLED
- script_sigops.rs:41:39 `> with >=` - EQUIV
- script_sigops.rs:44:15 `+= with -=` - still open
- scriptnum.rs:26:17 `< with >`
- scriptnum.rs:26:17 `< with <=`

## rbitcoin-consensus - 304 missed grouped

- block/mod.rs ~64: last_script_push, bip34_height_script, assemble_non_cb_tx, structural_bip68, is_final_tx
- silent_payments.rs ~52: is_p2pkh/p2sh/p2wpkh, witness_version
- confirm_run/pin.rs ~22, write.rs ~21
- script/interpreter.rs ~42: tapscript_has_op_success, eval_script, find_and_delete
- script_pool.rs 13 missed + 13 timeout
- signet.rs 11 missed + 8 timeout

Full log is 304 lines - kept as CI artifact / gist, not in this file.
