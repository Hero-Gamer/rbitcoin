use bitcoin::absolute::LockTime;
use bitcoin::hashes::Hash;
use bitcoin::transaction::Version as TxVersion;
use bitcoin::{Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness};
use futures_util::SinkExt;
use rbitcoin_net::{MempoolHub, TipEvent};
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message as WsMsg;

fn ws_spend(prev: [u8; 32], spk: ScriptBuf, value: u64) -> Transaction {
    Transaction {
        version: TxVersion::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint {
                txid: bitcoin::Txid::from_byte_array(prev),
                vout: 0,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(value),
            script_pubkey: spk,
        }],
    }
}

fn tip_header() -> bitcoin::block::Header {
    bitcoin::block::Header {
        version: bitcoin::block::Version::from_consensus(1),
        prev_blockhash: bitcoin::BlockHash::from_byte_array([0u8; 32]),
        merkle_root: bitcoin::TxMerkleNode::from_byte_array([0u8; 32]),
        time: 1,
        bits: bitcoin::CompactTarget::from_consensus(0x207fffff),
        nonce: 1,
    }
}

type Ws = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

async fn ws_protocol(
    ws: &mut Ws,
    q: &Query,
    tip_tx: &broadcast::Sender<TipEvent>,
    addr: std::net::SocketAddr,
    tip_height: u32,
) {
    let (st, body) = http_get(addr, "/blocks/tip/height").await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(body, tip_height.to_string());
    ws.send(WsMsg::Text(r#"{"action":"ping"}"#.into()))
        .await
        .unwrap();
    let ping = ws_recv_json(ws, 2).await;
    assert_eq!(ping["pong"], true, "{ping}");
    ws.send(WsMsg::Text(r#"{"action":"init"}"#.into()))
        .await
        .unwrap();
    let init = ws_recv_json(ws, 2).await;
    assert_eq!(init["block"]["height"], tip_height, "{init}");
    ws.send(WsMsg::Text(
        r#"{"action":"want","data":["blocks","stats"]}"#.into(),
    ))
    .await
    .unwrap();
    let stats = ws_recv_json(ws, 3).await;
    assert!(stats.get("mempoolInfo").is_some(), "{stats}");
    assert_eq!(stats["mempoolInfo"]["count"], 0, "{stats}");
    assert!(stats["fees"].get("fastestFee").is_some(), "{stats}");
    let tip_hash = q
        .header_at_height(Height(tip_height))
        .unwrap()
        .unwrap()
        .1
        .hash;
    let n = tip_tx
        .send(TipEvent {
            height: tip_height,
            hash: bitcoin::BlockHash::from_byte_array(tip_hash),
            header: tip_header(),
            reorg_branch_len: 0,
        })
        .expect("tip send");
    assert!(n >= 1, "expected at least one tip subscriber, got {n}");
    let mut saw_block = false;
    for _ in 0..6 {
        let v = ws_recv_json(ws, 3).await;
        if v.get("block").is_some() {
            assert_eq!(v["block"]["height"], tip_height);
            assert!(v["block"]["id"].as_str().unwrap().len() == 64);
            saw_block = true;
            break;
        }
    }
    assert!(saw_block, "want blocks pushes the tip");
    ws_reject_bad_address(ws).await;
    let (st, body) = http_get(addr, "/blocks/tip/height").await;
    assert_eq!(st, 200, "{body}");
}

async fn ws_reject_bad_address(ws: &mut Ws) {
    ws.send(WsMsg::Text(r#"{"track-address":"not-an-address"}"#.into()))
        .await
        .unwrap();
    let err = ws_recv_json(ws, 2).await;
    assert!(err.to_string().contains("invalid address"), "{err}");
    ws.send(WsMsg::Text(
        r#"{"track-addresses":["not-an-address"]}"#.into(),
    ))
    .await
    .unwrap();
    let err2 = ws_recv_json(ws, 2).await;
    assert!(err2.to_string().contains("invalid address"), "{err2}");
}

async fn ws_stats_bump(ws: &mut Ws, hub: &MempoolHub, coin: [u8; 32]) {
    let pay = ws_spend(coin, ScriptBuf::from_bytes(vec![0x51]), 49_0000_0000);
    hub.accept_tx(&pay).expect("admit");
    let mut saw_bump = false;
    for _ in 0..8 {
        let v = ws_recv_json(ws, 3).await;
        if let Some(info) = v.get("mempoolInfo") {
            assert!(info["count"].as_u64().unwrap_or(0) >= 1, "{v}");
            saw_bump = true;
            break;
        }
    }
    assert!(saw_bump, "expected mempoolInfo count bump after admit");
}

struct WsChain<'a> {
    q: &'a Query,
    hub: &'a MempoolHub,
    tip_tx: &'a broadcast::Sender<TipEvent>,
    coins: &'a [[u8; 32]],
    prev: Fk,
    parent: [u8; 32],
}

async fn ws_address_confirm(
    ws: &mut Ws,
    http: std::net::SocketAddr,
    watch_addr: &str,
    watch_spk: &bitcoin::ScriptBuf,
    chain: &WsChain<'_>,
) {
    let mut pay_bytes = [0u8; 32];
    pay_bytes[0] = 0xaa;
    pay_bytes[31] = 0xbb;
    let mut pay_disp = pay_bytes;
    pay_disp.reverse();
    let pay_hex = rbitcoin_primitives::hex_encode(pay_disp);
    let ta_pay = TxApply {
        tx: TxRecord {
            txid: pay_bytes,
            version: 2,
            locktime: 0,
            input_start_fk: Fk::NULL,
            input_count: 1,
            output_start_fk: Fk::NULL,
            output_count: 1,
        },
        inputs: vec![InputRecord {
            prev_txid: chain.coins[2],
            create_fk: Fk::NULL,
            prev_index: 0,
            sequence: 0xffff_fffd,
            script_sig: vec![],
            witness: vec![],
        }],
        outputs: vec![OutputRecord::unspent(49_0000_0000, watch_spk.to_bytes())],
    };
    let (cb_header, cb) = coinbase(110, chain.prev, Some(chain.parent));
    chain
        .q
        .connect_block(Height(110), &cb_header, &[cb, ta_pay])
        .expect("connect pay block");
    ws.send(WsMsg::Text(format!(r#"{{"track-address":"{watch_addr}"}}"#).into()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mem_pay = ws_spend(chain.coins[3], watch_spk.clone(), 49_0000_0000);
    let mem_hex = display_txid(mem_pay.compute_txid());
    chain.hub.accept_tx(&mem_pay).expect("mempool accept to watch");
    let (st, body) = http_get(http, &format!("/address/{watch_addr}/utxo")).await;
    assert_eq!(st, 200, "{body}");
    let utxos: Vec<serde_json::Value> = serde_json::from_str(&body).unwrap();
    assert!(utxos.iter().any(|u| u["status"]["confirmed"] == false), "{utxos:?}");
    assert!(utxos.iter().any(|u| u["status"]["confirmed"] == true), "{utxos:?}");
    let mut saw_addr_mp = false;
    for _ in 0..12 {
        let v = ws_recv_json(ws, 3).await;
        if let Some(arr) = v.get("address-transactions").and_then(|a| a.as_array()) {
            let txids: Vec<&str> = arr.iter().filter_map(|t| t["txid"].as_str()).collect();
            assert!(txids.iter().any(|t| *t == mem_hex), "{txids:?}");
            saw_addr_mp = true;
            break;
        }
    }
    assert!(saw_addr_mp, "expected address-transactions mempool push");
    chain
        .tip_tx
        .send(TipEvent {
            height: 110,
            hash: bitcoin::BlockHash::from_byte_array(cb_header.hash),
            header: tip_header(),
            reorg_branch_len: 0,
        })
        .unwrap();
    let mut saw_block_txs = false;
    for _ in 0..12 {
        let v = ws_recv_json(ws, 3).await;
        if let Some(arr) = v.get("block-transactions").and_then(|a| a.as_array()) {
            let txids: Vec<&str> = arr.iter().filter_map(|t| t["txid"].as_str()).collect();
            assert!(
                txids.contains(&pay_hex.as_str()),
                "block-transactions should include confirmed pay {pay_hex}, got {txids:?}"
            );
            saw_block_txs = true;
            break;
        }
    }
    assert!(saw_block_txs, "expected block-transactions at the pay tip");
}

async fn ws_snapshot_and_multi(
    ws: &mut Ws,
    hub: &MempoolHub,
    addr_a: &str,
    spk_a: &bitcoin::ScriptBuf,
    addr_b: &str,
    spk_b: &bitcoin::ScriptBuf,
    coins: &[[u8; 32]],
) {
    ws.send(WsMsg::Text(r#"{"stop-track-addresses":true}"#.into()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let old = ws_spend(coins[4], spk_a.clone(), 49_0000_0000);
    let old_hex = display_txid(old.compute_txid());
    hub.accept_tx(&old).expect("admit pay A");
    ws.send(WsMsg::Text(format!(r#"{{"track-address":"{addr_a}"}}"#).into()))
        .await
        .unwrap();
    let mut saw_snap = false;
    for _ in 0..12 {
        let v = ws_recv_json(ws, 3).await;
        if let Some(arr) = v.get("address-transactions").and_then(|a| a.as_array()) {
            let txids: Vec<&str> = arr.iter().filter_map(|t| t["txid"].as_str()).collect();
            assert!(txids.iter().any(|t| *t == old_hex), "{txids:?}");
            assert!(arr.iter().any(|t| t.get("vin").and_then(|x| x.as_array()).is_some()));
            saw_snap = true;
            break;
        }
    }
    assert!(saw_snap, "expected address-transactions snapshot on subscribe");
    let away = ws_spend(coins[4], ScriptBuf::from_bytes(vec![0x51]), 48_0000_0000);
    hub.accept_tx(&away).expect("rbf away from A");
    let mut saw_removed = false;
    for _ in 0..12 {
        let v = ws_recv_json(ws, 3).await;
        if let Some(arr) = v.get("address-removed-transactions").and_then(|a| a.as_array()) {
            let txids: Vec<&str> = arr.iter().filter_map(|t| t["txid"].as_str()).collect();
            assert!(txids.iter().any(|t| *t == old_hex), "{txids:?}");
            saw_removed = true;
            break;
        }
    }
    assert!(saw_removed, "expected address-removed-transactions on RBF");
    ws.send(WsMsg::Text(r#"{"stop-track-addresses":true}"#.into()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let pay_a = ws_spend(coins[5], spk_a.clone(), 49_0000_0000);
    let pay_b = ws_spend(coins[6], spk_b.clone(), 49_0000_0000);
    let hex_a = display_txid(pay_a.compute_txid());
    let hex_b = display_txid(pay_b.compute_txid());
    hub.accept_tx(&pay_a).expect("admit A");
    hub.accept_tx(&pay_b).expect("admit B");
    ws.send(WsMsg::Text(format!(r#"{{"track-addresses":["{addr_a}","{addr_b}"]}}"#).into()))
        .await
        .unwrap();
    let mut saw_multi = false;
    for _ in 0..12 {
        let v = ws_recv_json(ws, 3).await;
        if let Some(obj) = v.get("multi-address-transactions").and_then(|o| o.as_object()) {
            let ids_a: Vec<&str> = obj
                .get(addr_a)
                .and_then(|a| a.as_array())
                .map(|a| a.iter().filter_map(|t| t["txid"].as_str()).collect())
                .unwrap_or_default();
            let ids_b: Vec<&str> = obj
                .get(addr_b)
                .and_then(|a| a.as_array())
                .map(|a| a.iter().filter_map(|t| t["txid"].as_str()).collect())
                .unwrap_or_default();
            assert!(ids_a.iter().any(|t| *t == hex_a), "{ids_a:?}");
            assert!(ids_b.iter().any(|t| *t == hex_b), "{ids_b:?}");
            saw_multi = true;
            break;
        }
    }
    assert!(saw_multi, "expected multi-address-transactions keyed by display address");
}

async fn ws_track_tx_confirm(
    ws: &mut Ws,
    q: &Query,
    hub: &MempoolHub,
    tip_tx: &broadcast::Sender<TipEvent>,
    coin: [u8; 32],
) {
    let pending = ws_spend(coin, ScriptBuf::from_bytes(vec![0x51]), 49_0000_0000);
    let pending_hex = display_txid(pending.compute_txid());
    let pending_bytes = pending.compute_txid().to_byte_array();
    ws.send(WsMsg::Text(format!(r#"{{"track-tx":"{pending_hex}"}}"#).into()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    hub.accept_tx(&pending).expect("accept");
    let mut saw_unconf = false;
    for _ in 0..10 {
        let v = ws_recv_json(ws, 3).await;
        if v.get("tx").is_some() {
            assert_eq!(v["tx"]["txid"], pending_hex);
            assert_eq!(v["tx"]["status"]["confirmed"], false);
            saw_unconf = true;
            break;
        }
    }
    assert!(saw_unconf, "unconfirmed track-tx push");
    let (tip_fk, tip_rec) = q.header_at_height(Height(110)).unwrap().unwrap();
    let (h_hdr, cb) = coinbase(111, tip_fk, Some(tip_rec.hash));
    let ta = TxApply {
        tx: TxRecord {
            txid: pending_bytes,
            version: 2,
            locktime: 0,
            input_start_fk: Fk::NULL,
            input_count: 1,
            output_start_fk: Fk::NULL,
            output_count: 1,
        },
        inputs: vec![InputRecord {
            prev_txid: coin,
            create_fk: Fk::NULL,
            prev_index: 0,
            sequence: 0xffff_fffd,
            script_sig: vec![],
            witness: vec![],
        }],
        outputs: vec![OutputRecord::unspent(49_0000_0000, vec![0x51])],
    };
    q.connect_block(Height(111), &h_hdr, &[cb, ta]).expect("confirm pending");
    tip_tx
        .send(TipEvent {
            height: 111,
            hash: bitcoin::BlockHash::from_byte_array(h_hdr.hash),
            header: tip_header(),
            reorg_branch_len: 0,
        })
        .unwrap();
    let mut saw_conf = false;
    for _ in 0..12 {
        let v = ws_recv_json(ws, 3).await;
        if v.get("tx").is_some() && v["tx"]["txid"] == pending_hex && v["tx"]["status"]["confirmed"] == true {
            saw_conf = true;
            break;
        }
    }
    assert!(saw_conf, "expected confirmed track-tx status after tip");
}

async fn ws_rbf(ws: &mut Ws, hub: &MempoolHub, coin: [u8; 32]) {
    let low = ws_spend(coin, ScriptBuf::from_bytes(vec![0x51]), 50_0000_0000 - 1_000);
    let low_hex = display_txid(low.compute_txid());
    ws.send(WsMsg::Text(format!(r#"{{"track-tx":"{low_hex}"}}"#).into()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    hub.accept_tx(&low).unwrap();
    for _ in 0..5 {
        if let Ok(v) = tokio::time::timeout(Duration::from_millis(400), ws_recv_json(ws, 1)).await {
            if v.get("tx").is_some() {
                break;
            }
        } else {
            break;
        }
    }
    let high = ws_spend(coin, ScriptBuf::from_bytes(vec![0x51]), 50_0000_0000 - 10_000);
    let high_hex = display_txid(high.compute_txid());
    hub.accept_tx(&high).unwrap();
    let mut saw = false;
    for _ in 0..10 {
        let v = ws_recv_json(ws, 2).await;
        if let Some(arr) = v.get("replaced-transactions").and_then(|a| a.as_array()) {
            assert_eq!(arr[0]["txid"], low_hex);
            assert_eq!(arr[0]["replaced-by"], high_hex);
            saw = true;
            break;
        }
    }
    assert!(saw, "track-tx RBF replace frame");
}

async fn ws_rbf_away(
    ws: &mut Ws,
    hub: &MempoolHub,
    watch_addr: &str,
    watch_spk: &bitcoin::ScriptBuf,
    coin: [u8; 32],
) {
    ws.send(WsMsg::Text(r#"{"stop-track-txs":true}"#.into()))
        .await
        .unwrap();
    ws.send(WsMsg::Text(format!(r#"{{"track-address":"{watch_addr}"}}"#).into()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    let old_to_watch = ws_spend(coin, watch_spk.clone(), 49_0000_0000);
    let old_hex = display_txid(old_to_watch.compute_txid());
    hub.accept_tx(&old_to_watch).unwrap();
    for _ in 0..8 {
        if let Ok(v) = tokio::time::timeout(Duration::from_millis(400), ws_recv_json(ws, 1)).await {
            if v.get("address-transactions").is_some() {
                break;
            }
        } else {
            break;
        }
    }
    let repl_away = ws_spend(coin, ScriptBuf::from_bytes(vec![0x51]), 48_0000_0000);
    let repl_hex = display_txid(repl_away.compute_txid());
    hub.accept_tx(&repl_away).expect("rbf away from watch");
    let mut saw_addr_rbf = false;
    for _ in 0..12 {
        let v = ws_recv_json(ws, 2).await;
        if let Some(arr) = v.get("replaced-transactions").and_then(|a| a.as_array()) {
            assert_eq!(arr[0]["txid"], old_hex);
            assert_eq!(arr[0]["replaced-by"], repl_hex);
            saw_addr_rbf = true;
            break;
        }
    }
    assert!(saw_addr_rbf, "address-only RBF: old paid watch, new does not");
}

#[tokio::test]
async fn esplora_ws_track_address() {
    let (watch_addr, watch_spk) = regtest_p2wpkh();
    let (addr_b, spk_b) = regtest_p2wpkh_sk(8);
    let (dir, q) = temp_query("ws-track");
    let mut prev = Fk::NULL;
    let mut parent_hash: Option<[u8; 32]> = None;
    let mut coins = Vec::new();
    for h in 0..110u32 {
        let (header, ta) = coinbase(h, prev, parent_hash);
        parent_hash = Some(header.hash);
        coins.push(ta.tx.txid);
        prev = q.connect_block(Height(h), &header, &[ta]).unwrap();
    }
    let parent = parent_hash.unwrap();
    let q = Arc::new(q);
    let mp_dir = dir.join("mp");
    std::fs::create_dir_all(&mp_dir).unwrap();
    let hub = MempoolHub::open(&mp_dir, Arc::clone(&q)).unwrap();
    hub.set_relay_enabled(true);
    let (tip_tx, _) = broadcast::channel::<TipEvent>(16);
    let mut cfg = EsploraConfig::with_network("127.0.0.1:0".parse().unwrap(), bitcoin::Network::Regtest);
    cfg.max_ws_connections = 2;
    let handle = run_esplora(cfg, Arc::clone(&q), Some(Arc::clone(&hub)), Some(tip_tx.clone()))
        .await
        .expect("listen");
    let addr = handle.local_addr;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/v1/ws"))
        .await
        .expect("ws upgrade");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let chain = WsChain {
        q: &q,
        hub: &hub,
        tip_tx: &tip_tx,
        coins: &coins,
        prev,
        parent,
    };
    ws_protocol(&mut ws, &q, &tip_tx, addr, 109).await;
    ws_stats_bump(&mut ws, &hub, coins[0]).await;
    ws_address_confirm(&mut ws, addr, &watch_addr, &watch_spk, &chain).await;
    ws_snapshot_and_multi(&mut ws, &hub, &watch_addr, &watch_spk, &addr_b, &spk_b, &coins).await;
    ws_track_tx_confirm(&mut ws, &q, &hub, &tip_tx, coins[7]).await;
    ws_rbf(&mut ws, &hub, coins[8]).await;
    ws_rbf_away(&mut ws, &hub, &watch_addr, &watch_spk, coins[9]).await;

    let (mut ws2, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .expect("second ws");
    let third = tokio_tungstenite::connect_async(format!("ws://{addr}/v1/ws")).await;
    assert!(third.is_err(), "third upgrade should fail under max_ws=2");
    let _ = ws2.close(None).await;
    let _ = ws.close(None).await;
    handle.shutdown().await;
    let _ = std::fs::remove_dir_all(&dir);
}
