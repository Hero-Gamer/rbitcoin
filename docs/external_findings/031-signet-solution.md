# 031 — Signet solution bounds

**Severity:** high
**Status:** fixed
**Found by:** coordinated review, 2026-09-21 and 2026-09-22

The signet solution reader accepts only canonical CompactSize. A witness
item count larger than the bytes left is rejected before `with_capacity`.
PUSHDATA4 is parsed. Non-minimal pushes are re-encoded. A truncated push
after a parsed section keeps that section.

**Regression:** `rbitcoin-consensus` `signet::tests::compact_size_and_solution_parse_errors`,
`signet::tests::signet_section_pushdata4_and_minimal_reencode`
