# 042 — Milestone hash and minimum chain work

**Severity:** medium
**Status:** fixed
**Found by:** coordinated review, 2026-09-21 and 2026-09-22 (C14, Third #07, gist M-4)

Signet’s default milestone height is 0. Signet IBD runs every script.

The mainnet default is still height 840000, anchored to the block this
repository treats as mainnet block 840000
(`0000000000000000000320283a032748cef8227873ff4872689bf23f1cda83a5`,
Core assumeutxo). Script checks are skipped only when that hash occupies
the header path at 840000, the block under connect occupies the path at
its own height, and published header-path work is at least Core
`nMinimumChainWork`. A missing lookup does not skip. Explicit
`--milestone HEIGHT` stays a height-only skip. Omitted mainnet
`--min-chain-work` is that floor.

The gate is two height-keyed lookups plus the stored work total
(`ibd: perf_dbg milestone_us`). It does not walk ancestors. The path map
is a second copy of the IBD header path so confirm threads do not take
the IBD state lock.

**Regression:** `rbitcoin-consensus` `milestone::tests::low_work_fork_does_not_skip_even_at_the_milestone_height`,
`block::structure_rule_tests::p3_default_milestone_heights`,
`block::structure_rule_tests::buried_rules_and_a_lying_header_path`,
`rbitcoin-node` `cli::tests::default_milestone_is_anchored_and_signet_is_full_scripts`.
