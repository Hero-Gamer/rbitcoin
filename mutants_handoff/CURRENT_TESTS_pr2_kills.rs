// Current pr2_kills tests after last fix (from user's file)
#[test]
fn pr2_kills_zero_fee_and_zero_value() { /* mines 101 blocks, zero-fee tx */ }

#[test]
fn pr2_kills_negative_output_and_div_zero() { /* zero-value out valid */ }

#[test]
fn pr2_kills_sigops_boundary() {
    let cb = coinbase(0);
    let mut spend_ok = non_coinbase_spend(0);
    spend_ok.input[0].script_sig = ScriptBuf::from_bytes(vec![0xac; 20_000]);
    spend_ok.output[0].script_pubkey = ScriptBuf::from_bytes(vec![0xac; 20_000]);
    let block_ok = block_with(vec![cb.clone(), spend_ok]);
    validate_block_structure(&block_ok, &ctx_h(0)).expect("exactly 80k sigops must pass");
    let mut spend_over = non_coinbase_spend(0);
    spend_over.input[0].script_sig = ScriptBuf::from_bytes(vec![0xac; 20_001]);
    spend_over.output[0].script_pubkey = ScriptBuf::from_bytes(vec![0xac; 20_001]);
    let block_over = block_with(vec![cb, spend_over]);
    let err = validate_block_structure(&block_over, &ctx_h(0)).unwrap_err();
    assert_bad_block(err, "sigops");
}

#[test]
fn pr2_kills_lock_time_cutoff_genesis() { /* genesis uses block.time, height1 uses MTP */ }

#[test]
fn pr2_kills_value_out_max_money_boundary() {
    use super::assemble_tx_value_out;
    use rbitcoin_query::TxPrecompute;
    const MAX_MONEY: u64 = 21_000_000 * 100_000_000;
    let tx0 = Transaction { version: TxVersion::ONE, lock_time: LockTime::ZERO, input: vec![], output: vec![TxOut { value: Amount::ZERO, script_pubkey: ScriptBuf::from_bytes([0x51u8].to_vec()) }] };
    assert!(assemble_tx_value_out(&tx0, 0, None).is_ok());
    let tx1 = Transaction { version: TxVersion::ONE, lock_time: LockTime::ZERO, input: vec![], output: vec![TxOut { value: Amount::from_sat(1), script_pubkey: ScriptBuf::from_bytes([0x51u8].to_vec()) }] };
    assert_eq!(assemble_tx_value_out(&tx1, 0, None).unwrap(), 1, "must be 1 not 0 - kills Ok(0) mutant");
    let tx_max = Transaction { version: TxVersion::ONE, lock_time: LockTime::ZERO, input: vec![], output: vec![TxOut { value: Amount::from_sat(MAX_MONEY), script_pubkey: ScriptBuf::from_bytes([0x51u8].to_vec()) }] };
    assert!(assemble_tx_value_out(&tx_max, 0, None).is_ok());
    let tx_over = Transaction { version: TxVersion::ONE, lock_time: LockTime::ZERO, input: vec![], output: vec![TxOut { value: Amount::from_sat(MAX_MONEY+1), script_pubkey: ScriptBuf::from_bytes([0x51u8].to_vec()) }] };
    assert!(assemble_tx_value_out(&tx_over, 0, None).is_err(), "MAX+1 must be Err");
    let tx_sum_over = Transaction { version: TxVersion::ONE, lock_time: LockTime::ZERO, input: vec![], output: vec![TxOut { value: Amount::from_sat(MAX_MONEY), .. }, TxOut { value: Amount::from_sat(1), .. }] };
    assert!(assemble_tx_value_out(&tx_sum_over, 0, None).is_err());
    let pre = TxPrecompute::from_tx(&tx_over);
    let pres: Arc<[TxPrecompute]> = Arc::from([pre]);
    assert!(assemble_tx_value_out(&tx0, 0, Some(&pres)).is_err(), "pres over MAX must be Err - kills ||->&&");
}

#[test]
fn pr2_kills_ti_len_boundary() {
    let cb = coinbase(0);
    let spend = non_coinbase_spend(0);
    let block = block_with(vec![cb, spend]);
    validate_block_structure(&block, &ctx_h(0)).expect("ti<len boundary must be valid");
}
