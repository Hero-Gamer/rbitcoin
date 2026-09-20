# rbitcoin-consensus mutants (generated)

Owner: [`../../TESTING.md`](../../TESTING.md). Index: [`missed.md`](./missed.md).

Generated from 9h run: 304 missed + 38 timeouts = 342 entries

| File | Total | Missed | Timeout |
| :--- | ---: | ---: | ---: |
| `block/mod.rs` | 65 | 59 | 6 |
| `script/interpreter.rs` | 57 | 49 | 8 |
| `silent_payments.rs` | 50 | 46 | 4 |
| `confirm_run/write.rs` | 23 | 23 | 0 |
| `confirm_run/pin.rs` | 22 | 22 | 0 |
| `script_pool.rs` | 21 | 9 | 12 |
| `signet.rs` | 17 | 11 | 6 |
| `confirm_run/phases.rs` | 12 | 12 | 0 |
| `confirm_run/bq_resolve.rs` | 10 | 10 | 0 |
| `confirm_run/lookup.rs` | 10 | 10 | 0 |
| `header.rs` | 8 | 8 | 0 |
| `script/mod.rs` | 8 | 8 | 0 |
| `confirm_run/scripts.rs` | 8 | 8 | 0 |
| `convert.rs` | 6 | 6 | 0 |
| `policy.rs` | 5 | 5 | 0 |
| `params.rs` | 4 | 4 | 0 |
| `script/p2pkh.rs` | 4 | 4 | 0 |
| `confirm_run/mod.rs` | 3 | 3 | 0 |
| `regtest_pad.rs` | 2 | 0 | 2 |
| `script/classify.rs` | 2 | 2 | 0 |
| `script/p2wpkh.rs` | 2 | 2 | 0 |
| `lib.rs` | 1 | 1 | 0 |
| `error.rs` | 1 | 1 | 0 |
| `confirm_run/head_drain.rs` | 1 | 1 | 0 |

## block/mod.rs (65)

### line 314
- [ ] `delete ! in apply_witness_commitment`
- [ ] `replace || with && in apply_witness_commitment`

### line 357
- [ ] TIMEOUT - `replace += with *= in last_script_push`

### line 358
- [ ] TIMEOUT - `replace > with == in last_script_push`

### line 367
- [ ] TIMEOUT - `replace += with -= in last_script_push`
- [ ] TIMEOUT - `replace += with *= in last_script_push`

### line 380
- [ ] `replace + with * in last_script_push`

### line 383
- [ ] `replace + with - in last_script_push`

### line 385
- [ ] `replace > with >= in last_script_push`
- [ ] `replace > with < in last_script_push`

### line 391
- [ ] `replace + with * in last_script_push`
- [ ] `replace + with - in last_script_push`

### line 579
- [ ] `replace == with != in bip34_height_script`

### line 587
- [ ] `replace < with == in bip34_height_script`
- [ ] `replace < with > in bip34_height_script`
- [ ] `replace < with <= in bip34_height_script`

### line 591
- [ ] TIMEOUT - `replace > with >= in bip34_height_script`

### line 592
- [ ] TIMEOUT - `replace & with | in bip34_height_script`

### line 599
- [ ] `replace - with / in bip34_height_script`

### line 600
- [ ] `replace |= with &= in bip34_height_script`

### line 1119
- [ ] `replace == with != in assemble_lock_time_cutoff`

### line 1186
- [ ] `replace > with >= in assemble_non_cb_tx`
- [ ] `replace > with < in assemble_non_cb_tx`

### line 1190
- [ ] `replace > with >= in assemble_non_cb_tx`

### line 1194
- [ ] `replace - with + in assemble_non_cb_tx`
- [ ] `replace - with / in assemble_non_cb_tx`

### line 1204
- [ ] `replace < with == in assemble_non_cb_tx`
- [ ] `replace < with <= in assemble_non_cb_tx`
- [ ] `replace < with > in assemble_non_cb_tx`

### line 1225
- [ ] `replace < with == in assemble_tx_value_out`
- [ ] `replace < with > in assemble_tx_value_out`
- [ ] `replace < with <= in assemble_tx_value_out`

### line 1260
- [ ] `replace assemble_non_cb_inputs -> Result<(i64, Vec<TxOut>, u64), ConsensusError> with Ok((0, vec![], 0))`

### line 1681
- [ ] `replace != with == in structural_apply_one_meta`
- [ ] `replace & with | in structural_apply_one_meta`
- [ ] `replace & with ^ in structural_apply_one_meta`

### line 1726
- [ ] `replace structural_mark_pending -> Result<(), ConsensusError> with Ok(())`

### line 1802
- [ ] `delete ! in structural_bip68`

### line 1805
- [ ] `replace << with >> in structural_bip68`

### line 1806
- [ ] `replace << with >> in structural_bip68`

### line 1807
- [ ] `replace == with != in structural_bip68`

### line 1817
- [ ] `replace + with * in structural_bip68`

### line 1822
- [ ] `replace + with - in structural_bip68`

### line 1839
- [ ] `replace == with != in structural_bip68`
- [ ] `replace && with || in structural_bip68`
- [ ] `replace & with | in structural_bip68`
- [ ] `replace & with ^ in structural_bip68`
- [ ] `replace != with == in structural_bip68`
- [ ] `replace & with | in structural_bip68`
- [ ] `replace & with ^ in structural_bip68`

### line 1840
- [ ] `replace || with && in structural_bip68`
- [ ] `replace == with != in structural_bip68`
- [ ] `delete ! in structural_bip68`

### line 1940
- [ ] `replace || with && in job_needs_script_check`
- [ ] `delete ! in job_needs_script_check`
- [ ] `delete ! in job_needs_script_check`

### line 1984
- [ ] `replace == with != in is_final_tx`

### line 1987
- [ ] `replace < with == in is_final_tx`
- [ ] `replace < with > in is_final_tx`
- [ ] `replace < with <= in is_final_tx`

### line 1988
- [ ] `replace < with > in is_final_tx`
- [ ] `replace < with == in is_final_tx`

### line 2059
- [ ] `replace >= with < in sequence_locks_satisfied`

### line 2082
- [ ] `replace < with == in resolve_prevout`
- [ ] `replace < with <= in resolve_prevout`

## confirm_run/bq_resolve.rs (10)

### line 84
- [ ] `replace < with <= in bq_resolve_wave_hold_partial`
- [ ] `replace != with == in bq_resolve_wave_hold_partial`

### line 93
- [ ] `replace / with % in bq_resolve_wave_hold_partial`
- [ ] `replace > with >= in bq_resolve_wave_hold_partial`

### line 205
- [ ] `delete ! in confirm_bq_resolve_wave_capped`

### line 222
- [ ] `replace && with || in confirm_bq_resolve_wave_capped`

### line 223
- [ ] `replace < with <= in confirm_bq_resolve_wave_capped`
- [ ] `replace < with > in confirm_bq_resolve_wave_capped`

### line 253
- [ ] `delete ! in confirm_bq_resolve_wave_capped`

### line 266
- [ ] `delete ! in confirm_bq_resolve_wave_capped`

## confirm_run/head_drain.rs (1)

### line 102
- [ ] `replace <impl Drop for HeadDrainHandle>::drop with ()`

## confirm_run/lookup.rs (10)

### line 448
- [ ] `replace || with && in check_carried_header_lens`

### line 449
- [ ] `replace && with || in check_carried_header_lens`

### line 496
- [ ] `replace != with == in stamp_check_header_link`

### line 500
- [ ] `replace != with == in stamp_check_header_link`

### line 510
- [ ] `replace != with == in stamp_check_header_link`

### line 517
- [ ] `replace != with == in stamp_check_header_link`

### line 576
- [ ] `replace > with == in lookup_note_stamp`

### line 579
- [ ] `replace > with == in lookup_note_stamp`

### line 582
- [ ] `replace > with == in lookup_note_stamp`

### line 585
- [ ] `replace > with == in lookup_note_stamp`

## confirm_run/mod.rs (3)

### line 234
- [ ] `replace > with == in wire_blocks_to_arcs`

### line 349
- [ ] `replace ScriptOkBatch::parent_count -> usize with 1`
- [ ] `replace ScriptOkBatch::parent_count -> usize with 0`

## confirm_run/phases.rs (12)

### line 22
- [ ] `replace == with != in assemble_parent_mtp_and_bits`

### line 23
- [ ] `replace != with == in assemble_parent_mtp_and_bits`

### line 148
- [ ] `delete ! in assemble_run`
- [ ] `replace && with || in assemble_run`

### line 153
- [ ] `replace > with < in assemble_run`
- [ ] `replace > with == in assemble_run`
- [ ] `replace > with >= in assemble_run`

### line 185
- [ ] `replace > with == in assemble_run`

### line 197
- [ ] `delete ! in assemble_run`

### line 218
- [ ] `replace structural_run -> Result<crate::block::StructuralPhaseNs, ConsensusError> with Ok(Default::default())`

### line 224
- [ ] `replace - with + in structural_run`
- [ ] `replace - with / in structural_run`

## confirm_run/pin.rs (22)

### line 151
- [ ] `delete ! in fill_pins`

### line 157
- [ ] `replace > with == in fill_pins`

### line 173
- [ ] `replace && with || in apply_plan_pins`
- [ ] `delete ! in apply_plan_pins`

### line 176
- [ ] `replace != with == in apply_plan_pins`

### line 182
- [ ] `replace || with && in apply_plan_pins`

### line 190
- [ ] `delete ! in apply_plan_pins`

### line 194
- [ ] `replace != with == in apply_plan_pins`

### line 252
- [ ] `replace > with == in denserels_by_stamped_range`

### line 256
- [ ] `replace > with == in denserels_by_stamped_range`

### line 259
- [ ] `replace > with == in denserels_by_stamped_range`

### line 300
- [ ] `replace > with < in denserels_by_stamped_range`
- [ ] `replace > with == in denserels_by_stamped_range`

### line 341
- [ ] `delete ! in pin_for_wire_batch`

### line 358
- [ ] `replace > with == in pin_for_wire_batch`

### line 382
- [ ] `delete ! in pin_for_wire_batch`

### line 406
- [ ] `replace > with == in pin_for_wire_batch`

### line 410
- [ ] `replace > with == in pin_for_wire_batch`

### line 414
- [ ] `replace > with == in pin_for_wire_batch`

### line 417
- [ ] `replace > with == in pin_for_wire_batch`

### line 428
- [ ] `replace > with == in pin_for_wire_batch`

### line 434
- [ ] `replace > with == in pin_for_wire_batch`

## confirm_run/scripts.rs (8)

### line 26
- [ ] `delete ! in take_script_jobs`

### line 40
- [ ] `replace > with == in take_script_jobs`

### line 93
- [ ] `replace && with || in Inflight::is_complete`

### line 133
- [ ] `replace drive_script_waves_with::<impl Drop for ClearPublisher>::drop with ()`

### line 191
- [ ] `delete ! in drive_drain_complete`

### line 197
- [ ] `delete ! in drive_drain_complete`

### line 212
- [ ] `replace >= with < in drive_try_start`
- [ ] `replace || with && in drive_try_start`

## confirm_run/write.rs (23)

### line 170
- [ ] `replace > with == in confirm_write_phase`

### line 173
- [ ] `replace > with == in confirm_write_phase`

### line 176
- [ ] `replace > with == in confirm_write_phase`

### line 179
- [ ] `replace > with == in confirm_write_phase`

### line 189
- [ ] `replace > with == in confirm_write_phase`

### line 219
- [ ] `replace > with == in confirm_write_phase`

### line 240
- [ ] `replace > with == in confirm_write_phase`

### line 265
- [ ] `replace > with == in confirm_write_phase`

### line 460
- [ ] `replace || with && in fill_planned_create_layout_after_commit`
- [ ] `replace != with == in fill_planned_create_layout_after_commit`

### line 481
- [ ] `replace != with == in fill_planned_create_layout_after_commit`

### line 499
- [ ] `replace || with && in fill_planned_create_layout_after_commit`

### line 524
- [ ] `replace < with == in index_sp_tweaks_batch`
- [ ] `replace < with > in index_sp_tweaks_batch`
- [ ] `replace < with <= in index_sp_tweaks_batch`

### line 528
- [ ] `replace < with == in index_sp_tweaks_batch`

### line 534
- [ ] `replace > with < in index_sp_tweaks_batch`
- [ ] `replace > with >= in index_sp_tweaks_batch`

### line 586
- [ ] `replace != with == in records_from_wire`

### line 628
- [ ] `replace < with == in records_from_wire`
- [ ] `replace < with > in records_from_wire`
- [ ] `replace < with <= in records_from_wire`

### line 651
- [ ] `replace != with == in records_aligned_from_store`

## convert.rs (6)

### line 46
- [ ] `replace != with == in block_to_apply_with_txids`

### line 49
- [ ] `replace == with != in block_to_apply_with_txids`

### line 86
- [ ] `replace || with && in tx_to_apply`
- [ ] `replace == with != in tx_to_apply`

### line 87
- [ ] `replace && with || in tx_to_apply`
- [ ] `replace == with != in tx_to_apply`

## error.rs (1)

### line 24
- [ ] `replace <impl fmt::Display for ConsensusError>::fmt -> fmt::Result with Ok(Default::default())`

## header.rs (8)

### line 56
- [ ] `replace != with == in validate_header_hashed`

### line 62
- [ ] `replace != with == in validate_header_hashed`

### line 78
- [ ] `replace <= with > in validate_header_on_parent`

### line 82
- [ ] `replace != with == in validate_header_on_parent`

### line 155
- [ ] `replace > with < in median_time_past`
- [ ] `replace > with >= in median_time_past`

### line 222
- [ ] `replace - with + in expected_next_bits`
- [ ] `replace - with / in expected_next_bits`

## lib.rs (1)

### line 254
- [ ] `replace == with != in class_a_header_and_txids`

## params.rs (4)

### line 265
- [ ] `replace != with == in ChainParams::apply_test_activation_height`

### line 269
- [ ] `delete match arm "bip34" in ChainParams::apply_test_activation_height`

### line 272
- [ ] `delete match arm "csv" in ChainParams::apply_test_activation_height`

### line 273
- [ ] `delete match arm "segwit" in ChainParams::apply_test_activation_height`

## policy.rs (5)

### line 112
- [ ] `replace > with < in is_unspendable`
- [ ] `replace > with == in is_unspendable`
- [ ] `replace > with >= in is_unspendable`

### line 157
- [ ] `replace > with >= in check_libre_admission_at`

### line 160
- [ ] `delete ! in check_libre_admission_at`

## regtest_pad.rs (2)

### line 100
- [ ] TIMEOUT - `replace < with <= in coinbase_paying`
- [ ] TIMEOUT - `replace < with > in coinbase_paying`

## script/classify.rs (2)

### line 34
- [ ] `replace > with < in witness_program`
- [ ] `replace > with == in witness_program`

## script/interpreter.rs (57)

### line 111
- [ ] `delete match arm 0x4c in tapscript_has_op_success`

### line 112
- [ ] `replace >= with < in tapscript_has_op_success`

### line 118
- [ ] `delete match arm 0x4d in tapscript_has_op_success`

### line 125
- [ ] `delete match arm 0x4e in tapscript_has_op_success`

### line 137
- [ ] TIMEOUT - `replace += with *= in tapscript_has_op_success`
- [ ] TIMEOUT - `replace += with -= in tapscript_has_op_success`

### line 400
- [ ] `replace > with >= in eval_script`

### line 404
- [ ] `replace > with == in eval_script`

### line 521
- [ ] `delete ! in eval_script`

### line 527
- [ ] `delete match arm 0x00 in eval_script`

### line 534
- [ ] `delete match arm 0x61 in eval_script`

### line 535
- [ ] `delete match arm 0x62 in eval_script`

### line 538
- [ ] `delete match arm 0x65 | 0x66 in eval_script`

### line 541
- [ ] `delete match arm 0x69 in eval_script`

### line 547
- [ ] `delete match arm 0x6a in eval_script`

### line 549
- [ ] `delete match arm 0x6b in eval_script`

### line 665
- [ ] `replace > with < in eval_script`
- [ ] `replace > with == in eval_script`
- [ ] `replace + with - in eval_script`
- [ ] `replace > with >= in eval_script`

### line 872
- [ ] `replace != with == in eval_script`

### line 883
- [ ] `replace > with == in eval_script`
- [ ] `replace + with - in eval_script`
- [ ] `replace + with * in eval_script`

### line 898
- [ ] `replace << with >> in sequence_csv_ok`

### line 901
- [ ] `replace | with & in sequence_csv_ok`
- [ ] `replace | with ^ in sequence_csv_ok`
- [ ] `replace << with >> in sequence_csv_ok`

### line 1139
- [ ] `replace < with <= in find_and_delete`

### line 1141
- [ ] `replace <= with > in find_and_delete`

### line 1157
- [ ] TIMEOUT - `replace += with *= in find_and_delete`

### line 1162
- [ ] TIMEOUT - `replace += with -= in find_and_delete`
- [ ] TIMEOUT - `replace += with *= in find_and_delete`

### line 1165
- [ ] TIMEOUT - `replace == with != in find_and_delete`

### line 1174
- [ ] `replace + with * in find_and_delete`

### line 1177
- [ ] `replace + with - in find_and_delete`

### line 1182
- [ ] `replace + with * in find_and_delete`

### line 1185
- [ ] `replace + with - in find_and_delete`
- [ ] `replace + with - in find_and_delete`
- [ ] `replace + with * in find_and_delete`
- [ ] `replace + with - in find_and_delete`
- [ ] `replace + with * in find_and_delete`

### line 1187
- [ ] `replace + with - in find_and_delete`
- [ ] `replace + with * in find_and_delete`

### line 1188
- [ ] `replace += with -= in find_and_delete`
- [ ] `replace += with *= in find_and_delete`

### line 1193
- [ ] `replace > with == in find_and_delete`

### line 1224
- [ ] TIMEOUT - `replace += with *= in strip_op_codeseparator`

### line 1225
- [ ] TIMEOUT - `replace == with != in strip_op_codeseparator`

### line 1244
- [ ] `replace + with - in strip_op_codeseparator`
- [ ] `replace + with * in strip_op_codeseparator`

### line 1251
- [ ] `replace + with * in strip_op_codeseparator`
- [ ] `replace + with - in strip_op_codeseparator`

### line 1348
- [ ] `replace || with && in checksig_legacy_encodings`
- [ ] `replace || with && in checksig_legacy_encodings`

### line 1383
- [ ] `replace == with != in checksig_schnorr`

### line 1385
- [ ] `replace == with != in checksig_schnorr`

## script/mod.rs (8)

### line 296
- [ ] `replace - with / in crypto::parse_der_sig`

### line 311
- [ ] `replace crypto::is_valid_signature_encoding -> bool with true`

### line 344
- [ ] `replace == with != in crypto::is_valid_signature_encoding`
- [ ] `replace > with >= in crypto::is_valid_signature_encoding`

### line 351
- [ ] `replace == with != in crypto::is_valid_signature_encoding`
- [ ] `replace > with >= in crypto::is_valid_signature_encoding`

### line 386
- [ ] `replace match guard pk.len() == 65 with true in crypto::is_compressed_or_uncompressed_pubkey`
- [ ] `replace match guard pk.len() == 65 with false in crypto::is_compressed_or_uncompressed_pubkey`

## script/p2pkh.rs (4)

### line 51
- [ ] `replace && with || in parse_two_pushes`
- [ ] `replace >= with < in parse_two_pushes`
- [ ] `replace <= with > in parse_two_pushes`

### line 61
- [ ] `replace != with == in parse_two_pushes`

## script/p2wpkh.rs (2)

### line 21
- [ ] `replace != with == in verify`

### line 32
- [ ] `replace || with && in verify`

## script_pool.rs (21)

### line 65
- [ ] TIMEOUT - `delete ! in Wave::notify_if_complete`
- [ ] TIMEOUT - `replace Wave::notify_if_complete with ()`

### line 100
- [ ] TIMEOUT - `replace Wave::is_complete -> bool with false`

### line 101
- [ ] TIMEOUT - `replace || with && in Wave::is_complete`
- [ ] TIMEOUT - `replace >= with < in Wave::is_complete`

### line 127
- [ ] `replace Wave::has_unclaimed -> bool with false`
- [ ] TIMEOUT - `replace Wave::has_unclaimed -> bool with true`
- [ ] `delete ! in Wave::has_unclaimed`
- [ ] TIMEOUT - `replace && with || in Wave::has_unclaimed`
- [ ] `replace < with > in Wave::has_unclaimed`
- [ ] TIMEOUT - `replace < with == in Wave::has_unclaimed`
- [ ] `replace < with <= in Wave::has_unclaimed`

### line 132
- [ ] TIMEOUT - `delete ! in Wave::wait_done`

### line 146
- [ ] `replace unpark_script_publisher with ()`

### line 206
- [ ] TIMEOUT - `replace steal_bg_chunk -> Option<(Arc<Wave>, Range<usize>)> with None`

### line 217
- [ ] `replace help_steal -> bool with true`
- [ ] `replace help_steal -> bool with false`

### line 244
- [ ] `replace OwnedWave<T>::is_complete -> bool with false`
- [ ] TIMEOUT - `replace OwnedWave<T>::is_complete -> bool with true`

### line 360
- [ ] `replace == with != in run_wave`

### line 457
- [ ] TIMEOUT - `replace wake_steal_workers with ()`

## signet.rs (17)

### line 174
- [ ] TIMEOUT - `replace += with *= in fetch_and_clear_signet_section`

### line 175
- [ ] TIMEOUT - `replace == with != in fetch_and_clear_signet_section`

### line 204
- [ ] `replace += with *= in fetch_and_clear_signet_section`

### line 217
- [ ] `replace && with || in fetch_and_clear_signet_section`
- [ ] `replace < with > in fetch_and_clear_signet_section`
- [ ] `replace < with <= in fetch_and_clear_signet_section`
- [ ] `replace + with * in fetch_and_clear_signet_section`
- [ ] `replace + with - in fetch_and_clear_signet_section`

### line 224
- [ ] `replace += with *= in fetch_and_clear_signet_section`

### line 244
- [ ] `replace extract_header_payload -> Option<Vec<u8>> with None`

### line 264
- [ ] `replace < with <= in read_script`

### line 280
- [ ] `replace < with <= in read_witness_stack`

### line 294
- [ ] `replace modified_merkle_root -> Result<bitcoin::TxMerkleNode, ConsensusError> with Ok(Default::default())`

### line 299
- [ ] TIMEOUT - `replace > with < in modified_merkle_root`
- [ ] TIMEOUT - `replace > with == in modified_merkle_root`
- [ ] TIMEOUT - `replace > with >= in modified_merkle_root`

### line 300
- [ ] TIMEOUT - `replace == with != in modified_merkle_root`

## silent_payments.rs (50)

### line 46
- [ ] `replace secp -> &'static Secp256k1<All> with Box::leak(Box::new(Secp256k1::new()))`
- [ ] `replace secp -> &'static Secp256k1<All> with Box::leak(Box::new(Secp256k1::from(Default::default())))`

### line 145
- [ ] TIMEOUT - `delete ! in backfill_sp_tweaks_cancellable`

### line 180
- [ ] TIMEOUT - `replace > with == in backfill_sp_tweaks_cancellable`

### line 185
- [ ] TIMEOUT - `replace <= with > in backfill_sp_tweaks_cancellable`

### line 210
- [ ] TIMEOUT - `replace >= with < in backfill_sp_tweaks_cancellable`

### line 227
- [ ] `replace < with <= in maybe_log_sptweaks_backfill`

### line 233
- [ ] `replace / with % in maybe_log_sptweaks_backfill`
- [ ] `replace / with * in maybe_log_sptweaks_backfill`

### line 350
- [ ] `replace < with <= in taproot_outs_from_records`
- [ ] `replace < with > in taproot_outs_from_records`

### line 355
- [ ] `replace < with <= in taproot_outs_from_records`

### line 392
- [ ] `replace < with == in build_tx_and_prevouts`
- [ ] `replace < with > in build_tx_and_prevouts`
- [ ] `replace < with <= in build_tx_and_prevouts`

### line 404
- [ ] `replace < with <= in build_tx_and_prevouts`

### line 426
- [ ] `replace input_hash_mul_a -> Option<[u8; 33]> with None`

### line 429
- [ ] `replace < with <= in input_hash_mul_a`

### line 434
- [ ] `replace + with - in input_hash_mul_a`

### line 523
- [ ] `replace && with || in is_p2wpkh`
- [ ] `replace && with || in is_p2wpkh`

### line 528
- [ ] `replace && with || in is_p2pkh`

### line 529
- [ ] `replace && with || in is_p2pkh`

### line 530
- [ ] `replace && with || in is_p2pkh`

### line 531
- [ ] `replace && with || in is_p2pkh`

### line 532
- [ ] `replace && with || in is_p2pkh`

### line 536
- [ ] `replace && with || in is_p2sh`
- [ ] `replace && with || in is_p2sh`
- [ ] `replace && with || in is_p2sh`
- [ ] `replace == with != in is_p2sh`

### line 540
- [ ] `replace < with == in witness_version`
- [ ] `replace || with && in witness_version`
- [ ] `replace < with <= in witness_version`
- [ ] `replace < with > in witness_version`
- [ ] `replace > with < in witness_version`
- [ ] `replace > with == in witness_version`
- [ ] `replace > with >= in witness_version`

### line 544
- [ ] `delete match arm 0x00 in witness_version`

### line 545
- [ ] `delete match arm v @0x51..= 0x60 in witness_version`
- [ ] `replace - with + in witness_version`
- [ ] `replace - with / in witness_version`

### line 549
- [ ] `replace || with && in witness_version`
- [ ] `delete ! in witness_version`
- [ ] `replace != with == in witness_version`

### line 590
- [ ] `replace < with <= in nums_h_script_path`

### line 593
- [ ] `replace - with + in nums_h_script_path`

### line 594
- [ ] `replace < with <= in nums_h_script_path`
- [ ] `replace < with > in nums_h_script_path`

### line 667
- [ ] `replace && with || in is_p2sh_p2wpkh_redeem`
- [ ] `replace && with || in is_p2sh_p2wpkh_redeem`
