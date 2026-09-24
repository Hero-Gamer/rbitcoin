# 048 — Standard sigops and unscored invalid scripts

**Severity:** medium
**Status:** fixed
**Found by:** @otaliptus (M-5)

Mempool admission counted no standard sigop cap before the script
interpreter, and an invalid script was logged without a ban score.

A transaction whose sigop cost exceeds 16_000 (`MAX_BLOCK_SIGOPS_COST / 5`)
is rejected as `bad-txns-too-many-sigops` before `verify_tx_scripts_detached`.
An `AcceptError::Script` from a peer adds 10 to that peer's ban score.
Policy rejects, including the sigop cap, are not scored.

NULLFAIL, LOW_S, and CLEANSTACK stay at the flags production script verify
already uses: NULLFAIL and LOW_S are off, and witness programs already
require a clean true stack. Block validation does not gain those flags.

**Regression:** `rbitcoin-mempool`
`accept::tests::too_many_sigops_rejected_before_script`,
`rbitcoin-net` `peer::tests::invalid_script_is_scored_and_policy_is_not`.
