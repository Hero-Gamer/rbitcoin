use rbitcoin_store::script_hash;

async fn cap_refuses_three_creates(q: &Query, app: &Router) {
    let h51 = block_hash_hex(&script_hash(&[0x51]));
    q.set_max_sh_creates(2);
    let (st, body) = oneshot_http(app, get_with_client(&format!("/scripthash/{h51}"), "cap")).await;
    assert_eq!(st, 503, "{body}");
    assert!(
        body.contains("scripthash join exceeds --max-sh-creates"),
        "{body}"
    );
    q.set_max_sh_creates(0);
}

async fn casa_reuses_last_slot(app: &Router, cache: &Mutex<JoinCache>) {
    let sh = script_hash(&[0x51]);
    let h51 = block_hash_hex(&sh);
    let (st, body) = oneshot_http(app, get_with_client(&format!("/scripthash/{h51}"), "casa")).await;
    assert_eq!(st, 200, "{body}");
    let info: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(info["chain_stats"]["tx_count"], 3);
    let (st, utxo) =
        oneshot_http(app, get_with_client(&format!("/scripthash/{h51}/utxo"), "casa")).await;
    assert_eq!(st, 200, "{utxo}");
    let utxos: Value = serde_json::from_str(&utxo).unwrap();
    assert_eq!(utxos.as_array().map(|a| a.len()), Some(3), "{utxo}");
    let (st, txs) =
        oneshot_http(app, get_with_client(&format!("/scripthash/{h51}/txs"), "casa")).await;
    assert_eq!(st, 200, "{txs}");
    assert_eq!(cache.lock().unwrap().last_sh_key("casa"), Some(sh));
}

async fn two_addresses_each_have_one_utxo(app: &Router, a1: &str, a2: &str) {
    let p1 = format!("/address/{a1}/utxo");
    let p2 = format!("/address/{a2}/utxo");
    let ((st1, b1), (st2, b2)) = tokio::join!(
        oneshot_http(app, get_with_client(&p1, "a")),
        oneshot_http(app, get_with_client(&p2, "b")),
    );
    assert_eq!(st1, 200, "{b1}");
    assert_eq!(st2, 200, "{b2}");
    let v1: Value = serde_json::from_str(&b1).unwrap();
    let v2: Value = serde_json::from_str(&b2).unwrap();
    assert_eq!(v1.as_array().map(|a| a.len()), Some(1), "{b1}");
    assert_eq!(v2.as_array().map(|a| a.len()), Some(1), "{b2}");
}

async fn last1_and_second_client(app: &Router, cache: &Mutex<JoinCache>, sh1: [u8; 32], sh2: [u8; 32]) {
    let h1hex = block_hash_hex(&sh1);
    let h2hex = block_hash_hex(&sh2);
    let (st, body) = oneshot_http(app, get_with_client(&format!("/scripthash/{h1hex}"), "c1")).await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(cache.lock().unwrap().last_sh_key("c1"), Some(sh1));
    let (st, body) =
        oneshot_http(app, get_with_client(&format!("/scripthash/{h1hex}/utxo"), "c1")).await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(cache.lock().unwrap().last_sh_key("c1"), Some(sh1));
    let (st, body) = oneshot_http(app, get_with_client(&format!("/scripthash/{h2hex}"), "c1")).await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(
        cache.lock().unwrap().last_sh_key("c1"),
        Some(sh2),
        "GET B replaces last-1"
    );
    let (st, body) = oneshot_http(app, get_with_client(&format!("/scripthash/{h1hex}"), "c2")).await;
    assert_eq!(st, 200, "{body}");
    {
        let g = cache.lock().unwrap();
        assert_eq!(g.last_sh_key("c1"), Some(sh2));
        assert_eq!(g.last_sh_key("c2"), Some(sh1));
    }
}

async fn bulk_reuses_wallet_slot(
    q: Arc<Query>,
    app: &Router,
    cache: Arc<Mutex<JoinCache>>,
    sh1: [u8; 32],
    sh2: [u8; 32],
) {
    let h1hex = block_hash_hex(&sh1);
    let h2hex = block_hash_hex(&sh2);
    let t2 = block_hash_hex(&[
        0x22, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0xaa,
    ]);
    let body = serde_json::to_vec(&json!([&h1hex, &h2hex])).unwrap();
    let req = axum::http::Request::builder()
        .method("POST")
        .uri("/scripthashes/txs")
        .header("X-Rbitcoin-Client", "wallet")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.clone()))
        .unwrap();
    let (st, resp) = oneshot_http(app, req).await;
    assert_eq!(st, 200, "{resp}");
    assert_eq!(cache.lock().unwrap().bulk_len("wallet"), 2);
    let st = join_only_state(q, Arc::clone(&cache));
    let mut bag = HashMap::new();
    st.seed_bulk(Some("wallet"), &mut bag);
    assert_eq!(bag.len(), 2);
    {
        let g = cache.lock().unwrap();
        let c = g.clients.get("wallet").expect("wallet bulk");
        let packed: usize = c.last_bulk.values().map(|s| s.packed_bytes()).sum();
        assert!(packed <= JOIN_BULK_CAP, "last-bulk stays under 16 MiB");
        for (k, v) in &bag {
            let cached = c.last_bulk.get(k).expect("seeded key");
            assert!(Arc::ptr_eq(cached, v), "seed_bulk must Arc-clone last-bulk");
        }
    }
    let req = axum::http::Request::builder()
        .method("POST")
        .uri(format!("/scripthashes/txs?after_txid={t2}"))
        .header("X-Rbitcoin-Client", "wallet")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body))
        .unwrap();
    let (st, resp) = oneshot_http(app, req).await;
    assert_eq!(st, 200, "{resp}");
    assert_eq!(cache.lock().unwrap().bulk_len("wallet"), 2);
}

async fn untrusted_header_is_ignored(q: Arc<Query>, sh1: [u8; 32]) {
    let h1hex = block_hash_hex(&sh1);
    let public = Arc::new(Mutex::new(JoinCache::default()));
    let pub_app = app_with_join(Arc::clone(&q), Arc::clone(&public), false);
    let plain = axum::http::Request::builder()
        .uri(format!("/scripthash/{h1hex}"))
        .body(axum::body::Body::empty())
        .unwrap();
    let (st_plain, body_plain) = oneshot_http(&pub_app, plain).await;
    let (st, body) =
        oneshot_http(&pub_app, get_with_client(&format!("/scripthash/{h1hex}"), "ignored")).await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(st_plain, st);
    assert_eq!(
        body_plain, body,
        "public TCP does not read X-Rbitcoin-Client"
    );
    assert!(public.lock().unwrap().last_sh_key("ignored").is_none());

    let loopback = Arc::new(Mutex::new(JoinCache::default()));
    let lb_app = app_with_join(q, Arc::clone(&loopback), false);
    let mut req = get_with_client(&format!("/scripthash/{h1hex}"), "lb");
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1))));
    let (st, body) = oneshot_http(&lb_app, req).await;
    assert_eq!(st, 200, "{body}");
    assert!(
        loopback.lock().unwrap().last_sh_key("lb").is_none(),
        "loopback without join_header_trusted ignores X-Rbitcoin-Client"
    );
}

async fn template_cached_until_tip_moves(q: Arc<Query>, prev: Fk, parent: [u8; 32]) {
    let calls = Arc::new(AtomicU64::new(0));
    let c = Arc::clone(&calls);
    let mut cfg = EsploraConfig::with_network("127.0.0.1:0".parse().unwrap(), Network::Regtest);
    cfg.block_template = Some(BlockTemplateFn(Arc::new(move || {
        c.fetch_add(1, Ordering::Relaxed);
        Ok(json!({"height": 1, "rules": ["segwit"]}))
    })));
    let handle = run_esplora(cfg, Arc::clone(&q), None, None)
        .await
        .expect("listen");
    let (st, raw, body) = http_get_raw(handle.local_addr, "/block-template").await;
    assert_eq!(st, 200, "{body}");
    assert!(
        raw.to_ascii_lowercase().contains("cache-control: no-store"),
        "{raw}"
    );
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["height"], 1);
    let (st, _) = http_get(handle.local_addr, "/block-template").await;
    assert_eq!(st, 200);
    assert_eq!(calls.load(Ordering::Relaxed), 1, "15s cache");
    let (h3, t3) = coinbase(3, prev, Some(parent));
    q.connect_block(Height(3), &h3, &[t3]).unwrap();
    let (st, _) = http_get(handle.local_addr, "/block-template").await;
    assert_eq!(st, 200);
    assert_eq!(calls.load(Ordering::Relaxed), 2, "tip change invalidates");
    handle.shutdown().await;

    let mut cfg = EsploraConfig::new("127.0.0.1:0".parse().unwrap());
    cfg.block_template = Some(BlockTemplateFn(Arc::new(|| Err("no hub".into()))));
    let handle = run_esplora(cfg, q, None, None).await.expect("listen");
    let (st, body) = http_get(handle.local_addr, "/block-template").await;
    assert_eq!(st, 503, "{body}");
    assert!(body.contains("no hub"), "{body}");
    handle.shutdown().await;
}

async fn template_missing_tip_or_knob() {
    let (dir, empty) = temp_query("gbt-notip");
    let mut cfg = EsploraConfig::new("127.0.0.1:0".parse().unwrap());
    cfg.block_template = Some(BlockTemplateFn(Arc::new(|| Ok(json!({"height": 1})))));
    let handle = run_esplora(cfg, Arc::new(empty), None, None)
        .await
        .expect("listen");
    let (st, body) = http_get(handle.local_addr, "/block-template").await;
    assert_eq!(st, 503, "{body}");
    handle.shutdown().await;
    let _ = std::fs::remove_dir_all(&dir);

    let (dir, q) = temp_query("gbt-404");
    let cfg = EsploraConfig::new("127.0.0.1:0".parse().unwrap());
    let handle = run_esplora(cfg, Arc::new(q), None, None)
        .await
        .expect("listen");
    let (st, body) = http_get(handle.local_addr, "/block-template").await;
    assert_eq!(st, 404, "{body}");
    handle.shutdown().await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn esplora_sh_join_and_template() {
    let (a1, spk1) = regtest_p2wpkh();
    let (a2, spk2) = regtest_p2wpkh_sk(9);
    let sh1 = script_hash(spk1.as_bytes());
    let sh2 = script_hash(spk2.as_bytes());
    let (dir, q) = temp_query("sh-join");
    let (h0, t0) = coinbase(0, Fk::NULL, None);
    let prev = q.connect_block(Height(0), &h0, &[t0]).unwrap();
    let (h1, cb1) = coinbase(1, prev, Some(h0.hash));
    let prev = q.connect_block(Height(1), &h1, &[cb1]).unwrap();
    let (h2, cb2) = coinbase(2, prev, Some(h1.hash));
    let prev = q
        .connect_block(
            Height(2),
            &h2,
            &[
                cb2,
                two_script_pay(0x11, spk1),
                two_script_pay(0x22, spk2),
            ],
        )
        .unwrap();
    let q = Arc::new(q);
    let cache = Arc::new(Mutex::new(JoinCache::default()));
    let app = app_with_join(Arc::clone(&q), Arc::clone(&cache), true);
    cap_refuses_three_creates(&q, &app).await;
    casa_reuses_last_slot(&app, &cache).await;
    two_addresses_each_have_one_utxo(&app, &a1, &a2).await;
    last1_and_second_client(&app, &cache, sh1, sh2).await;
    bulk_reuses_wallet_slot(Arc::clone(&q), &app, Arc::clone(&cache), sh1, sh2).await;
    untrusted_header_is_ignored(Arc::clone(&q), sh1).await;
    template_cached_until_tip_moves(q, prev, h2.hash).await;
    template_missing_tip_or_knob().await;
    let _ = std::fs::remove_dir_all(&dir);
}
