// From user's sed -n '1180,1250p' crates/rbitcoin-consensus/src/block/mod.rs
    let tx_legacy_sigops = match pres.and_then(|p| p.get(ti)) {
        Some(p) => p.sigops.saturating_mul(4),
        None => legacy_sigop_count(tx).saturating_mul(4),
    };
    *block_sigops_cost = block_sigops_cost
        .checked_add(tx_legacy_sigops)
        .and_then(|c| c.checked_add(tx_in_sigops))
        .ok_or(ConsensusError::BadBlock("bad-blk-sigops"))?;
    if *block_sigops_cost > MAX_BLOCK_SIGOPS_COST { // 1188 MISSED > with ==
        return Err(ConsensusError::BadBlock("bad-blk-sigops"));
    }
    let value_out = assemble_tx_value_out(tx, ti, pres)?;
    if value_out > value_in {
        return Err(ConsensusError::BadTx("in < out"));
    }
    let fee = value_in.checked_sub(value_out).ok_or(ConsensusError::BadTx("fee overflow"))?;
    if fee < 0 {
        return Err(ConsensusError::BadTx("negative fee"));
    }
    *fees = fees.checked_add(fee).ok_or(ConsensusError::BadTx("fee overflow"))?;
    if build_script_jobs {
        let t_job = Instant::now();
        let mut job = if let Some(w) = wire {
            ScriptCheckJob::with_shared_tx(txid, prevouts, Arc::clone(w), ti, flags)
        } else {
            ScriptCheckJob::with_txid(txid, prevouts, tx.clone(), flags)
        };
        if let Some(ps) = pres {
            if ti < ps.len() { // 1208 MISSED
                job = job.with_pre_slice(Arc::clone(ps), ti);
            }
        }
        script_jobs.push(job);
        *clk_job = clk_job.saturating_add(t_job.elapsed().as_nanos() as u64);
    }
    Ok(())
}

fn assemble_tx_value_out(
    tx: &Transaction,
    ti: usize,
    pres: Option<&Arc<[rbitcoin_query::TxPrecompute]>>,
) -> Result<i64, ConsensusError> {
    const MAX_MONEY: i64 = 21_000_000 * 100_000_000;
    match pres.and_then(|p| p.get(ti)) {
        Some(p) => {
            let sum = p.out_sum as i64;
            if sum < 0 || sum > MAX_MONEY { // 1227 MISSED < with ==, || with &&, > with >=
                return Err(ConsensusError::BadTx("value out of range"));
            }
            Ok(sum)
        },
        None => {
            let mut value_out = 0i64;
            for o in &tx.output {
                let sats_u64 = o.value.to_sat();
                if sats_u64 > MAX_MONEY as u64 { // 1236 MISSED
                    return Err(ConsensusError::BadTx("output value too large"));
                }
                let sats = sats_u64 as i64;
                value_out = value_out
                    .checked_add(sats)
                    .ok_or(ConsensusError::BadTx("value out overflow"))?;
                if value_out > MAX_MONEY { // 1244
                    return Err(ConsensusError::BadTx("tx output sum too large"));
                }
            }
            Ok(value_out)
        }
    }
}
