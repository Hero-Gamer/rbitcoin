# External findings (fuzzamoto / differential / redteam)

Consensus, P2P, and mempool issues reported against rbitcoin (Bitcoin Core primary vs
rbitcoin reference, or redteam static analysis). Numbered reports live beside this index.

| ID | Severity | Topic | Status | Regression (shipped) |
|----|----------|--------|--------|----------------------|
| [001](./001-disconnect-on-invalid-block.md) | medium | Peer disconnect on invalid relayed block (BIP-152) | fixed | `rbitcoin-net` `peer::tests::cmpct_helpers_without_mempool_and_queue_out_closed` |
| [002](./002-store-corrupt-record-on-invalid-block.md) | low | Invalid block misclassified as store corrupt | fixed | `rbitcoin-consensus` `error::tests::archive_unresolved_parent_is_missing_prevout_not_corrupt` |
| [003](./003-bip68-version-signedness-consensus-split.md) | high | BIP68 skipped for version with bit 31 set | fixed | `block::tests::bip68_enforced_when_version_high_bit_set` |
| [004](./004-csv-nop-and-scriptnum-width.md) | high | CSV v1 no-op; CLTV/CSV 4-byte scriptnum | fixed | `script::interpreter::tests::csv_fails_when_tx_version_below_2` + Core script corpus |
| [005](./005-non-topological-block-accepted.md) | high | Non-topological same-block spends accepted | fixed | `rbitcoin-test` `consensus_rules::header_and_spending_boundaries` |
| [006](./006-p2sh-scriptsig-push-size.md) | medium | P2SH scriptSig pushes not limited to 520 bytes | fixed | `script::nested::tests::p2sh_scriptsig_push_over_520_rejected` |
| [007](./007-p2sh-nested-witness-exactness.md) | medium | P2SH nested-witness scriptSig exactness / program rules | fixed | `script::nested` nested-witness malleation tests |
| [008](./008-p2tr-keypath-sighash-zero.md) | medium | P2TR key-path 65-byte sig with sighash byte 0x00 | fixed | `script::p2tr::tests::key_path_rejects_65_byte_sighash_byte_zero` |
| [009](./009-witness-commitment-reserved.md) | medium | Witness commitment empty/multi-item coinbase witness | fixed | `block::tests::s8_rejects_empty_or_multi_item_coinbase_witness_reserved` |
| [010](./010-mempool-confirmed-spentness.md) | medium | Mempool no confirmed-chain spentness check | fixed | `rbitcoin-mempool` `accept::tests::reject_when_provider_has_no_unspent_coin` |
| [011](./011-mempool-structural-chain-context.md) | medium | Mempool no structural chain-context validation | fixed | `accept::tests::reject_non_final_locktime_height`, `reject_immature_coinbase` |
| [012](./012-p2sh-redeem-not-executed.md) | high | P2SH redeem skipped when BIP16 looks off | fixed | `bip16_from_prev_mtp_exception_and_time` |
| [013](./013-bip68-unresolved-age-fail-open.md) | high | BIP68 unresolved coin age fails open | fixed | `bip68_unresolved_coin_age_fails_closed` |
| [014](./014-stranded-on-peer-reorg.md) | high | Stranded when peer reorgs (sync) | fixed | `drain_requests_missing_parent_of_pending_branch` |
| [015](./015-spend-rejected-block-outputs.md) | high | Spend outputs of a rejected block | fixed | cluster 017/019 + structural fail-closed |
| [016](./016-unknown-taproot-leaf-rejected.md) | critical | Unknown tapleaf version rejected | fixed | `script_path_accepts_unknown_taproot_leaf_version` |
| [017](./017-duplicate-txid-unconnected-instance.md) | medium | Txid resolve hits unconnected instance | fixed | `resolve_txid_prefers_connected_over_newer_unconnected` |
| [018](./018-compact-block-duplicate-tx.md) | high | Compact block duplicates a tx | fixed | `repeated_short_id_is_requested_not_duplicated` |
| [019](./019-bip30-not-enforced.md) | critical | BIP30 not enforced | fixed | cluster 015/017 + BIP34-gated batch |
| [020](./020-pending-child-after-reorg.md) | high | Pending child not connected after reorg | fixed | `drain_connects_pending_child_of_new_tip_after_reorg` |
| [021](./021-regtest-activation-heights.md) | low | Regtest BIP65/66 heights stale | fixed | `params::tests::for_network_and_helpers` |
| [022](./022-stack-altstack-share-max-size.md) | high | `MAX_STACK_SIZE` ignored altstack on PushBytes / TUCK | fixed | `stack_and_altstack_share_max_size_on_pushdata` |
| [023](./023-tapscript-initial-stack-limits.md) | high | Tapscript initial witness stack skipped 1000/520 limits | fixed | `script_path_rejects_initial_stack_over_max_size` |
| [038](./038-tip-accept-lifetime.md) | high | Async tip-accept job must not borrow a dropped hub | fixed | `owned_job_finishes_after_waiter_abort` |
| [036](./036-electrum-public-surface.md) | critical | Public Electrum scan secret, join, and subscription caps | fixed | `paged_history_stops_before_the_create_cap` |
| [035](./035-bip30-bip34-ancestry.md) | low | BIP30 skipped on signet after height 1 | fixed | `bip30_signet_rejects_unspent_overwrite_after_bip34` |
| [034](./034-witness-padding-not-cached.md) | high | Witness padding cached as an invalid block hash | fixed | `padded_coinbase_witness_over_weight_is_not_cached_invalid` |
| [033](./033-ibd-intake-bounds.md) | critical | IBD path state and body bytes before validation | fixed | `rejected_header_batch_does_not_grow_path_or_explore` |
| [030](./030-header-accept.md) | critical | Invalid header held; zero prev wipes tip | fixed | `zero_prev_with_live_tip_is_not_held` |
| [029](./029-compact-tx-count.md) | critical | Compact block tx count and one partial per peer | fixed | `reconstruct_rejects_tx_count_above_weight_ratio`, `pending_header_insert_past_cap_clears` |
| [032](./032-block-tx-count-alloc.md) | critical | Block tx count allocated before the payload was checked | fixed | `decode_block_precomputes_rejects_tx_count_past_payload` |
| [031](./031-signet-solution.md) | high | Signet solution CompactSize and pushes | fixed | `compact_size_and_solution_parse_errors` |
| [024](./024-talip-review-index.md) | — | Talip review index | index | Q-71 |
| [025](./025-tapleaf-annex-prefix.md) | critical | Tapleaf `0x50` rejected | fixed | `script_path_accepts_leaf_0x50_with_annex_and_0xc2` |
| [026](./026-false-witness-program.md) | high | False witness program accepted | fixed | `false_witness_program_is_eval_false` |
| [027](./027-script-skip-wtxid.md) | critical | Script skip keyed by txid | fixed | `tip_script_pres_skips_only_matching_wtxid` |
| [028](./028-prevout-count.md) | high | Empty prevouts skipped script checks | fixed | `prevout_count_must_match_inputs` |
| [042](./042-milestone-anchor.md) | medium | Height milestone skipped a low-work fork | fixed | `low_work_fork_does_not_skip_even_at_the_milestone_height` |
| [041](./041-hygiene.md) | medium | P2PKH policy flags, RPC token, inv cap, log newlines | fixed | `peer_command_logs_have_no_raw_newline` |
| [037](./037-rpc-esplora-limits.md) | high | RPC body, work queue, waits, and Esplora client id | fixed | `unauthorized_large_content_length_is_401_before_body` |
| [039](./039-io-lifetimes.md) | high | Store I/O buffer must outlive the submit | fixed | `enter_failure_with_pending_matches_the_hard_cap` |
| [047](./047-orphan-reserve.md) | medium | Orphan reserve can refuse every later orphan | fixed | `mempool_under_pressure` |
| [048](./048-standard-sigops.md) | medium | No standard sigop cap before the interpreter | fixed | `mempool_under_pressure` |
| [049](./049-rolling-min-fee.md) | medium | Full-mempool fee floor is a static bump | fixed | `mempool_under_pressure` |
| [050](./050-cluster-once.md) | low | Cluster rebuild once per input | fixed | `two_parent_insert_builds_the_cluster_once` |
| [051](./051-rbfr-direct-set.md) | low | RBFR uses the direct conflict set | won't-fix | `pure_rbfr_unpins_descendant_package` |
| [046](./046-spend-durability.md) | high | Spend slot missing after the tip seal is unspent | fixed | `zeroed_spend_slot_after_tip_seal_rejects_respend` |
| [045](./045-addr-relay.md) | medium | Addr relay is one or two peers, not every peer | fixed | `addrv2_reaches_one_or_two_neighbors_and_stops_at_the_burst` |
| [044](./044-local-auth.md) | low | Tor SAFECOOKIE, datadir `0700`, socket mode, overlay permissions | fixed | `tor_plain_cookie_is_not_sent_when_safecookie_is_absent` |
| [043](./043-peer-send-buffer.md) | high | Unbounded per-peer outbound queue | fixed | `getheaders_flood_stops_at_the_send_budget_and_getaddr_is_once` |
| [040](./040-corrupt-bounds.md) | medium | Corrupt uleb128, seqsigwit lengths, and BDZ modulus | fixed | `read_packed_zero_modulus_or_vertices_is_corrupt` |

**012–021:** fuzzamoto differential report (`rbitcoin-report.tar.gz`, baseline
`8f3990f`). Report-local 001–010 are **renumbered** here. Identity/BIP30
(015/017/019) is one cluster (`TipOnly` confirm lookup + fail-closed height).

**Policy:** Core `script_tests` / `tx_valid` / `tx_invalid` corpora must pass **every**
data row with **no allowlist**. Do not commit if those tests fail. Findings stay
**fixed** with a named regression on the shipped path. A green regression means
the write-up stays closed. Do not re-derive the bug from the narrative unless
that test fails.

**006–009:** consensus accept-invalid (zip 2026-08-10) — **fixed** in-tree. **010–011:**
mempool remediation **fixed** (Coin spentness + structural tip checks + fee path).

Remediation (2026-08): 001–005 fixed in-tree; see each file **Status** and **Regression**.
