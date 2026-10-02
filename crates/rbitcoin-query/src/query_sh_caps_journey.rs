fn sh_page(
    q: &Query,
    sh: &[u8; 32],
    order: crate::scripthash::HistoryOrder,
    after_txid: Option<[u8; 32]>,
) -> Result<Vec<[u8; 32]>, QueryError> {
    let filter = crate::scripthash::HistoryFilter {
        limit: Some(1),
        order,
        after_txid,
        ..crate::scripthash::HistoryFilter::open()
    };
    Ok(q.scripthash_history_filtered(sh, &filter)?
        .iter()
        .map(|i| i.txid)
        .collect())
}

/// One scripthash with more creates than `--max-sh-creates`: the full join
/// refuses, a page that closes before the cap is still served in either
/// order and past a cursor, and the create count includes the write-behind
/// that has not reached the durable index yet.
#[test]
fn sh_history_caps() {
    use crate::scripthash::HistoryOrder::{HeightAsc, NewestFirst};

    let (dir, q) = temp_query("sh-history-caps");
    let sh = script_hash(&[0x51]);
    let probe_sh = script_hash(&[0x52]);
    let add_probe_output = |ta: &mut TxApply| {
        ta.tx.output_count += 1;
        ta.outputs
            .push(OutputRecord::unspent(1, vec![0x52]));
    };
    let mut prev = Fk::NULL;
    let mut parent = None;
    let mut cb_txids = Vec::new();
    for h in 0..5u32 {
        let (header, mut ta) = coinbase_block(h, prev, parent);
        add_probe_output(&mut ta);
        parent = Some(header.hash);
        cb_txids.push(ta.tx.txid);
        prev = q.connect_block(Height(h), &header, &[ta]).unwrap();
    }
    let create0 = q.block_tx_fks(Height(0)).unwrap()[0];

    let (h5, mut cb5) = coinbase_block(5, prev, parent);
    add_probe_output(&mut cb5);
    let mut spend = cb5.clone();
    spend.tx.output_count = 1;
    spend.outputs.truncate(1);
    spend.tx.txid[30] = 0x5e;
    spend.inputs = vec![InputRecord {
        prev_txid: cb_txids[0],
        create_fk: create0,
        prev_index: 0,
        sequence: u32::MAX,
        script_sig: vec![],
        witness: vec![],
    }];
    let spend_txid = spend.tx.txid;
    q.commit_class_a_only(&h5, &[cb5, spend]).unwrap();
    q.confirm_block(Height(5), &h5.hash).unwrap();
    assert_eq!(q.pending_sh_create_fks(&sh).len(), 2);
    assert_eq!(
        q.scripthash_create_count(&sh).unwrap(),
        7,
        "the create count includes the pending write-behind"
    );
    q.apply_sh_pending().unwrap();
    assert!(q.pending_sh_create_fks(&sh).is_empty());
    assert_eq!(q.scripthash_create_count(&sh).unwrap(), 7);
    let tip_coinbase_fk = q.block_tx_fks(Height(5)).unwrap()[0];

    q.set_max_sh_creates(2);
    for err in [
        q.scripthash_chain_stats(&sh).map(|_| ()).unwrap_err(),
        q.scripthash_history(&sh).map(|_| ()).unwrap_err(),
    ] {
        assert!(
            matches!(err, StoreError::Rejected(m) if m == Query::MAX_SH_CREATES_MSG),
            "{err}"
        );
    }
    assert_eq!(
        sh_page(&q, &sh, HeightAsc, None).unwrap(),
        [cb_txids[0]],
        "an ascending page that closes before the cap is served"
    );
    assert_eq!(
        sh_page(&q, &sh, HeightAsc, Some(cb_txids[0])).unwrap(),
        [cb_txids[1]],
        "a full page past the cursor closes too"
    );
    q.store().reset_txid_get_many();
    assert_eq!(
        sh_page(&q, &probe_sh, HeightAsc, Some(cb_txids[3])).unwrap(),
        [cb_txids[4]],
        "a cursor deeper than the cap still returns its next row"
    );
    let scanned = q.store().txid_get_many_fks();
    assert!(
        !scanned.contains(&tip_coinbase_fk.0),
        "once the cursor and full page are joined, later creates cannot change the page"
    );
    assert_eq!(
        sh_page(&q, &sh, NewestFirst, None).unwrap(),
        [spend_txid],
        "the newest row spends the oldest create"
    );

    let view = q.pin_sh_chain_view().unwrap().expect("sh view");
    let page = crate::scripthash::HistoryFilter::esplora_chain_page(None);
    let mut slot = None;
    let rows = q
        .scripthash_history_filtered_slot_in(&sh, &page, &mut slot, &view)
        .expect("paged history skips the unpaged cap");
    assert!(!rows.is_empty());
    assert!(
        slot.is_none(),
        "a paged miss must not fill the join the cap refuses"
    );
    let sums = q
        .scripthash_history_summary_filtered_slot_in(&sh, &page, &mut slot, &view)
        .expect("paged summary skips the unpaged cap");
    assert!(!sums.is_empty());
    assert!(slot.is_none());
    let err = q
        .scripthash_history_filtered_slot_in(
            &sh,
            &crate::scripthash::HistoryFilter::open(),
            &mut slot,
            &view,
        )
        .unwrap_err();
    assert!(
        matches!(err, StoreError::Rejected(m) if m == Query::MAX_SH_CREATES_MSG),
        "{err}"
    );

    q.set_max_sh_creates(0);
    assert_eq!(q.scripthash_history(&sh).unwrap().len(), 7);
    q.set_max_sh_creates(7);
    let stats = q.scripthash_chain_stats(&sh).unwrap();
    assert_eq!((stats.funded_txo_count, stats.spent_txo_count), (7, 1));
    let _ = std::fs::remove_dir_all(&dir);
}
