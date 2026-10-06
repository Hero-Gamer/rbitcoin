# 086 — P2SH spend skipped the scriptPubKey stack limit

**Severity:** high
**Status:** fixed
**Found by:** consensus audit, 2026-10-06

Core's `VerifyScript` always runs the scriptPubKey on the stack that
scriptSig leaves. For P2SH that script is `HASH160 <20> EQUAL`, and it
runs before the push-only check and the redeemScript. `EvalScript`
checks `stack + altstack > MAX_STACK_SIZE` (1000) after every opcode.
`HASH160` keeps the depth, the `<20>` push adds one item, and `EQUAL`
takes it back. So a scriptSig that leaves exactly 1000 items fails
with `SCRIPT_ERR_STACK_SIZE`. The scriptSig run caps its own result at
1000, so this is the only depth that fails.

After BIP16 we did not run the scriptPubKey. We only compared
`HASH160(redeem)` with the 20 bytes in it. A 1000-item P2SH spend was
accepted. Before BIP16 the spend goes through the bare path, which
runs the scriptPubKey and already rejected it.

Example: redeemScript `OP_1`, scriptPubKey
`a914 <hash160(0x51)> 87`, scriptSig `999 × OP_0` then `01 51`:

| scriptSig items | Core | rbitcoin before |
|-----------------|------|-----------------|
| 999 | valid | valid |
| 1000 | invalid (`STACK_SIZE`) | valid |

Checked against libbitcoinconsensus 25.1: 1000 items is rejected even
with no verify flags, 999 items is accepted. A miner could put the
1000-item spend in a block that Core rejects and rbitcoin connects.

The other ways `HASH160 <20> EQUAL` can fail were already covered. An
empty stack is rejected on both P2SH paths, and a hash mismatch is the
redeem hash check. The script is a fixed 23 bytes with two counted
opcodes, so script size and op count cannot fail. Nested segwit needs a
single-push scriptSig, so its stack holds one item.

`p2sh_script_sig_stack` now rejects with `stack size` when scriptSig
leaves `MAX_STACK_SIZE` items, at the point where Core runs the
scriptPubKey. Both the nested and the legacy P2SH paths use that stack.
The depth check replaces a real scriptPubKey run, so there is no stack
clone and no second `HASH160` per P2SH input.

**Regression:** consensus matrix row C35, `rbitcoin-consensus`
`script::tests_verify::p2sh_script_pubkey_push_counts_against_max_stack_size`
