# 026 — False witness program accepted

**Severity:** high
**Status:** fixed
**Found by:** coordinated review, 2026-09-22

An unknown-version witness program of all zeros (or negative zero) was
anyone-can-spend. Core's scriptPubKey leaves the program on the stack
and fails `CastToBool` before witness success. The same check applies
to a P2SH-wrapped program.

**Regression:** `rbitcoin-consensus`
`script::verify_routing_tests::false_witness_program_is_eval_false`
