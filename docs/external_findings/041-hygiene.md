# 041 — P2PKH policy flags, RPC token, inv cap, and log newlines

**Severity:** medium
**Status:** fixed
**Found by:** coordinated review, 2026-09-21 and 2026-09-22 (Third #14, C17, Third #19)

A P2PKH job with `LOW_S`, `STRICTENC`, or `NULLFAIL` uses the generic
interpreter. The fast path does not apply those flags. With the flags
off, the fast path is unchanged. Consensus confirm still leaves them off.

RPC bearer compare walks both byte strings. A new token file is created
mode 0600; create does not chmod afterwards.

`inv`, `getdata`, and `notfound` whose count is above 50,000 fail decode
(`MessageTooLarge`). The handlers drop an oversize inventory instead of
processing a prefix. A log line built from a peer command replaces raw
newlines.

**Regression:** `rbitcoin-consensus` `script::tests_verify::mainnet_block_140493_high_bit_s_lax_der_p2pkh`,
`rbitcoin-rpc` `auth::tests::bearer_parse_and_token_paths`,
`auth::tests::token_file_is_created_owner_only`,
`rbitcoin-net` `codec::tests::core_limits_documented`,
`peer::log_text_tests::peer_command_logs_have_no_raw_newline`.
