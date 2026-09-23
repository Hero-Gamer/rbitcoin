# 025 — Tapleaf 0x50 rejected

**Severity:** critical
**Status:** fixed
**Found by:** coordinated review, 2026-09-22

`ControlBlock::decode` refuses leaf version `0x50`. Core hashes
`control[0] & 0xfe` and accepts that leaf when the commitment matches.
A future leaf is not executed. Tapscript (`0xc0`) still is.

**Regression:** `rbitcoin-consensus`
`script::p2tr::bip341_tests::script_path_accepts_leaf_0x50_with_annex_and_0xc2`
