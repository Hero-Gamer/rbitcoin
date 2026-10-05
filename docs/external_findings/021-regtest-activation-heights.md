# Regtest CLTV / strict DER activation heights may be stale

**Component:** `ChainParams::regtest` / rust-bitcoin inherited heights
**Audit pin:** fuzzamoto report 010 / rbitcoin `8f3990f`
**Severity:** low — regtest only
**Status:** fixed — `ChainParams::regtest` sets bip34/bip65/bip66 = 1
**Found by:** fuzzamoto

## Summary

Core regtest sets BIP65 and BIP66 to height **1**. rust-bitcoin historically
used 1351 / 1251, so CLTV is a no-op and DER is loose for the whole useful test
range. Signet in-tree already asserts height 1; regtest must be checked.

BIP34 on regtest had the same gap: rust-bitcoin carries 100_000_000, Core
`CRegTestParams` uses `BIP34Height = 1` with an empty `BIP34Hash`. A height-1
coinbase without the height push was accepted where Core rejects
`bad-cb-height`.

## Fix

If still stale, set `bip65_height = 1`, `bip66_height = 1` on `ChainParams::regtest`.
Add a low-height CLTV-unsatisfied reject test if missing.

BIP34 follow-up: `ChainParams::regtest` sets `bip34_height = 1`.
`-testactivationheight=bip34@N` still moves it. `bip34_hash` stays `None`, so
BIP30 stays enforced on regtest like Core. In-tree regtest miners already emit
the height push and nVersion 4.

**Regression:** `rbitcoin-consensus`
`block::structure_rule_tests::s7_regtest_rejects_bip34_missing_at_height_1`,
`s7_regtest_bip34_activation_height_override`,
`params::tests::bip34_hash_gates_bip30_like_core`; `rbitcoin-rpc`
`rpc_regtest_from_genesis` (`generatetoaddress` at height 1, `getdeploymentinfo`
bip34 active).
