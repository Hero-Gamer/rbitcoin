//! One `run_p2p` process: Esplora broadcast is visible on RPC and Electrum.

use bitcoin::absolute::LockTime;
use bitcoin::consensus::Encodable;
use bitcoin::hashes::Hash;
use bitcoin::script::ScriptBuf;
use bitcoin::transaction::Version as TxVersion;
use bitcoin::{Amount, OutPoint, Sequence, Transaction, TxIn, TxOut, Txid, Witness};
use rbitcoin_consensus::{accept_and_connect_block, pad_empty_from, ChainParams, Milestone};
use rbitcoin_electrum::electrum_scripthash_hex;
use rbitcoin_node::{run_p2p, NodeConfig};
use rbitcoin_primitives::{Height, Network};
use rbitcoin_query::Query;
use rbitcoin_test::TestDatadir;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

/// HTTP Basic for `user:pass` (same pair NodeConfig uses below).
const RPC_BASIC: &str = "Basic dXNlcjpwYXNz";

fn ephemeral_addr() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    addr
}

async fn wait_listeners(addrs: &[SocketAddr]) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let mut missing = None;
        for addr in addrs {
            if TcpStream::connect(addr).await.is_err() {
                missing = Some(*addr);
                break;
            }
        }
        if missing.is_none() {
            return;
        }
        if Instant::now() >= deadline {
            panic!("listeners not up ({missing:?}): {addrs:?}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn http_exchange(addr: SocketAddr, req: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).await.expect("http connect");
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf).into_owned();
    let status = text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = text
        .split("\r\n\r\n")
        .nth(1)
        .unwrap_or("")
        .trim()
        .to_string();
    (status, body)
}

async fn http_get(addr: SocketAddr, path: &str) -> (u16, String) {
    http_exchange(
        addr,
        &format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"),
    )
    .await
}

async fn http_post(addr: SocketAddr, path: &str, body: &str) -> (u16, String) {
    http_exchange(
        addr,
        &format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
    )
    .await
}

async fn jsonrpc(addr: SocketAddr, method: &str, params: Value) -> Value {
    let body = json!({"jsonrpc":"1.0","id":"test","method":method,"params":params}).to_string();
    let req = format!(
        "POST / HTTP/1.1\r\nHost: {addr}\r\nAuthorization: {RPC_BASIC}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let (_st, text) = http_exchange(addr, &req).await;
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("rpc {method} json: {e} body={text}"))
}

async fn electrum_rpc(stream: &mut TcpStream, id: u64, method: &str, params: Value) -> Value {
    let req = json!({"jsonrpc":"2.0","id": id, "method": method, "params": params});
    let mut line = serde_json::to_string(&req).unwrap();
    line.push('\n');
    stream.write_all(line.as_bytes()).await.unwrap();
    let mut reader = BufReader::new(&mut *stream);
    let mut resp_line = String::new();
    tokio::time::timeout(Duration::from_secs(5), reader.read_line(&mut resp_line))
        .await
        .unwrap_or_else(|_| panic!("electrum {method}: read_line timed out"))
        .unwrap_or_else(|e| panic!("electrum {method}: io {e}"));
    serde_json::from_str(&resp_line).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn esplora_broadcast_visible_in_rpc_and_electrum() {
    let td = TestDatadir::new().unwrap();
    let params = ChainParams::regtest();
    let genesis = bitcoin::blockdata::constants::genesis_block(bitcoin::Network::Regtest);
    let (coinbase_txid, rpc_cb) = {
        let q = Query::open_or_create_tiny(td.store_path()).unwrap();
        accept_and_connect_block(&q, &params, Height::GENESIS, &genesis, Milestone::NONE).unwrap();
        let (_tip, _time, cbs) = pad_empty_from(
            &q,
            &params,
            genesis.block_hash(),
            genesis.header.time,
            1,
            102,
            2,
        );
        q.flush().unwrap();
        (cbs[0], cbs[1])
    };

    let electrum_addr = ephemeral_addr();
    let esplora_addr = ephemeral_addr();
    let rpc_addr = ephemeral_addr();

    let mut cfg = NodeConfig::default()
        .with_datadir(td.path())
        .with_network(Network::Regtest)
        .with_tiny_heads()
        .with_p2p_listen("127.0.0.1:0".parse().unwrap());
    cfg.listen.use_seeds = false;
    cfg.listen.connect.clear();
    cfg.shindex = true;
    cfg.listen.electrum = Some(electrum_addr);
    cfg.listen.esplora = Some(esplora_addr);
    cfg.rpc.listen = Some(rpc_addr);
    cfg.rpc.user = Some("user".into());
    cfg.rpc.password = Some("pass".into());
    cfg.max_run_secs = Some(60);

    let node = tokio::spawn(run_p2p(cfg));
    wait_listeners(&[electrum_addr, esplora_addr, rpc_addr]).await;

    let (st, height) = http_get(esplora_addr, "/blocks/tip/height").await;
    assert_eq!(st, 200, "esplora tip height: {height}");
    assert_eq!(height, "102");
    let count = jsonrpc(rpc_addr, "getblockcount", json!([])).await;
    assert_eq!(count["result"], 102, "{count}");

    let rpc_spk = ScriptBuf::from_bytes(vec![0x54]);
    let rpc_spend = Transaction {
        version: TxVersion::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint {
                txid: rpc_cb,
                vout: 0,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(50_0000_0000 - 1_000),
            script_pubkey: rpc_spk,
        }],
    };
    let mut rpc_raw = Vec::new();
    rpc_spend.consensus_encode(&mut rpc_raw).unwrap();
    let rpc_hex = rbitcoin_primitives::hex_encode(&rpc_raw);
    let rpc_txid = rpc_spend.compute_txid().to_string();
    let mut zero_fee = rpc_spend.clone();
    zero_fee.output[0].value = Amount::from_sat(50_0000_0000);
    let mut zero_raw = Vec::new();
    zero_fee.consensus_encode(&mut zero_raw).unwrap();
    let zero_hex = rbitcoin_primitives::hex_encode(&zero_raw);
    let tma = jsonrpc(rpc_addr, "testmempoolaccept", json!([[zero_hex]])).await;
    assert_eq!(tma["result"][0]["allowed"], false, "{tma}");
    assert_eq!(
        tma["result"][0]["reject-reason"], "min relay fee not met",
        "{tma}"
    );
    let tma = jsonrpc(rpc_addr, "testmempoolaccept", json!([[rpc_hex.clone()]])).await;
    assert_eq!(tma["result"][0]["allowed"], true, "{tma}");
    let sent = jsonrpc(rpc_addr, "sendrawtransaction", json!([rpc_hex])).await;
    assert_eq!(sent["result"], rpc_txid, "{sent}");
    let mem = jsonrpc(rpc_addr, "getrawmempool", json!([])).await;
    let ids = mem["result"].as_array().expect("getrawmempool array");
    assert!(
        ids.iter().any(|v| v.as_str() == Some(rpc_txid.as_str())),
        "getrawmempool missing sendraw {rpc_txid}: {mem}"
    );
    let miss = Transaction {
        version: TxVersion::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint {
                txid: Txid::from_byte_array([0x11; 32]),
                vout: 0,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(1),
            script_pubkey: ScriptBuf::from_bytes(vec![0x51]),
        }],
    };
    let mut miss_raw = Vec::new();
    miss.consensus_encode(&mut miss_raw).unwrap();
    let miss_hex = rbitcoin_primitives::hex_encode(&miss_raw);
    let tma = jsonrpc(rpc_addr, "testmempoolaccept", json!([[miss_hex]])).await;
    assert_eq!(tma["result"][0]["allowed"], false, "{tma}");
    assert_eq!(
        tma["result"][0]["reject-reason"], "bad-txns-inputs-missingorspent",
        "{tma}"
    );

    let spk = ScriptBuf::from_bytes(vec![0x52]);
    let spend = Transaction {
        version: TxVersion::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint {
                txid: coinbase_txid,
                vout: 0,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(50_0000_0000 - 1_000),
            script_pubkey: spk.clone(),
        }],
    };
    let mut raw = Vec::new();
    spend.consensus_encode(&mut raw).unwrap();
    let hex = rbitcoin_primitives::hex_encode(&raw);
    let txid_hex = spend.compute_txid().to_string();

    let (st, body) = http_post(esplora_addr, "/tx", &hex).await;
    assert_eq!(st, 200, "POST /tx: {body}");
    assert_eq!(body, txid_hex);
    let dup = jsonrpc(rpc_addr, "sendrawtransaction", json!([hex.clone()])).await;
    assert_eq!(dup["result"], txid_hex, "sendraw of live mempool tx: {dup}");

    let (st, status) = http_get(esplora_addr, &format!("/tx/{txid_hex}/status")).await;
    assert_eq!(st, 200, "GET /tx status: {status}");
    let status_v: Value = serde_json::from_str(&status).unwrap();
    assert_eq!(status_v["confirmed"], false, "{status_v}");

    let mem = jsonrpc(rpc_addr, "getrawmempool", json!([])).await;
    let ids = mem["result"].as_array().expect("getrawmempool array");
    assert!(
        ids.iter().any(|v| v.as_str() == Some(txid_hex.as_str())),
        "getrawmempool missing {txid_hex}: {mem}"
    );
    assert!(
        ids.iter().any(|v| v.as_str() == Some(rpc_txid.as_str())),
        "getrawmempool dropped sendraw {rpc_txid}: {mem}"
    );

    let sh = electrum_scripthash_hex(spk.as_bytes());
    let mut el = TcpStream::connect(electrum_addr).await.unwrap();
    let _ = electrum_rpc(&mut el, 1, "server.version", json!(["test", "1.4"])).await;
    let mempool = electrum_rpc(&mut el, 2, "blockchain.scripthash.get_mempool", json!([sh])).await;
    let mem_rows = mempool["result"].as_array().expect("get_mempool array");
    let mem_row = mem_rows
        .iter()
        .find(|r| r["tx_hash"] == txid_hex)
        .unwrap_or_else(|| panic!("get_mempool missing {txid_hex}: {mempool}"));
    let fee = mem_row["fee"].as_i64().expect("get_mempool fee");
    assert!(fee > 0, "get_mempool fee: {mem_row}");

    let hist = electrum_rpc(&mut el, 3, "blockchain.scripthash.get_history", json!([sh])).await;
    let hist_row = hist["result"]
        .as_array()
        .expect("get_history array")
        .iter()
        .find(|r| r["tx_hash"] == txid_hex)
        .unwrap_or_else(|| panic!("get_history missing {txid_hex}: {hist}"));
    assert_eq!(hist_row["fee"].as_i64(), Some(fee), "{hist_row}");
    for row in hist["result"].as_array().unwrap() {
        if row["tx_hash"] == txid_hex {
            assert!(row.get("fee").is_some(), "unconfirmed history fee: {row}");
        } else {
            assert!(
                row.get("fee").is_none(),
                "confirmed history omits fee: {row}"
            );
        }
    }

    let child_spk = ScriptBuf::from_bytes(vec![0x53]);
    let child = Transaction {
        version: TxVersion::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint {
                txid: spend.compute_txid(),
                vout: 0,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(50_0000_0000 - 2_000),
            script_pubkey: child_spk.clone(),
        }],
    };
    let mut child_raw = Vec::new();
    child.consensus_encode(&mut child_raw).unwrap();
    let child_hex = rbitcoin_primitives::hex_encode(&child_raw);
    let child_txid = child.compute_txid().to_string();
    let (st, body) = http_post(esplora_addr, "/tx", &child_hex).await;
    assert_eq!(st, 200, "POST /tx child: {body}");
    assert_eq!(body, child_txid);

    let child_sh = electrum_scripthash_hex(child_spk.as_bytes());
    let child_hist = electrum_rpc(
        &mut el,
        4,
        "blockchain.scripthash.get_history",
        json!([child_sh.clone()]),
    )
    .await;
    let child_row = child_hist["result"]
        .as_array()
        .expect("child history array")
        .iter()
        .find(|r| r["tx_hash"] == child_txid)
        .unwrap_or_else(|| panic!("child history missing {child_txid}: {child_hist}"));
    assert_eq!(child_row["height"], -1, "{child_row}");
    let child_fee = child_row["fee"].as_i64().expect("mempool child fee");
    assert!(child_fee > 0, "mempool child fee: {child_row}");

    let child_mem = electrum_rpc(
        &mut el,
        5,
        "blockchain.scripthash.get_mempool",
        json!([child_sh]),
    )
    .await;
    let child_mem_row = child_mem["result"]
        .as_array()
        .expect("child mempool array")
        .iter()
        .find(|r| r["tx_hash"] == child_txid)
        .unwrap_or_else(|| panic!("get_mempool missing child {child_txid}: {child_mem}"));
    assert_eq!(
        child_mem_row["fee"].as_i64(),
        Some(child_fee),
        "{child_mem_row}"
    );

    let mined = jsonrpc(rpc_addr, "generate", json!([1])).await;
    assert_eq!(
        mined["result"].as_array().map(|a| a.len()),
        Some(1),
        "{mined}"
    );
    let count = jsonrpc(rpc_addr, "getblockcount", json!([])).await;
    assert_eq!(count["result"], 103, "{count}");
    let empty = jsonrpc(rpc_addr, "getrawmempool", json!([])).await;
    assert_eq!(empty["result"], json!([]), "{empty}");
    let tip = jsonrpc(rpc_addr, "getbestblockhash", json!([])).await;
    let blk = jsonrpc(rpc_addr, "getblock", json!([tip["result"].clone(), 2])).await;
    let txs = blk["result"]["tx"].as_array().expect("mined tx array");
    assert!(
        txs.len() >= 4,
        "coinbase + sendraw + esplora parent + child: {blk}"
    );
    assert!(
        txs.iter().any(|t| t["txid"] == rpc_txid),
        "generate must include sendraw: {blk}"
    );
    let cb_txid = txs[0]["txid"].as_str().expect("coinbase txid").to_string();
    let cb_val = (txs[0]["vout"][0]["value"].as_f64().unwrap() * 100_000_000.0).round() as u64;
    let immature = Transaction {
        version: TxVersion::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint {
                txid: Txid::from_str(&cb_txid).expect("coinbase txid"),
                vout: 0,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(cb_val.saturating_sub(1_000)),
            script_pubkey: ScriptBuf::from_bytes(vec![0x51]),
        }],
    };
    let mut imm_raw = Vec::new();
    immature.consensus_encode(&mut imm_raw).unwrap();
    let imm = jsonrpc(
        rpc_addr,
        "sendrawtransaction",
        json!([rbitcoin_primitives::hex_encode(&imm_raw)]),
    )
    .await;
    assert_eq!(imm["error"]["code"], -26, "{imm}");
    assert_eq!(
        imm["error"]["message"], "bad-txns-premature-spend-of-coinbase",
        "{imm}"
    );

    let _ = jsonrpc(rpc_addr, "stop", json!([])).await;
    let stopped = tokio::time::timeout(Duration::from_secs(15), node).await;
    match stopped {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(e))) => panic!("run_p2p error after stop: {e}"),
        Ok(Err(e)) => panic!("run_p2p join: {e}"),
        Err(_) => panic!("run_p2p did not exit after stop"),
    }
}
