//! End of IBD: tip follow, scripthash history, and restart on a live miner.

use bitcoin::absolute::LockTime;
use bitcoin::script::ScriptBuf;
use bitcoin::transaction::Version as TxVersion;
use bitcoin::{Amount, OutPoint, Sequence, Transaction, TxIn, TxOut, Txid, Witness};
use rbitcoin_consensus::{ChainParams, Milestone};
use rbitcoin_electrum::electrum_scripthash_hex;
use rbitcoin_net::{IbdConfig, NetAddr, P2PNode};
use rbitcoin_node::{run_p2p, NodeConfig};
use rbitcoin_primitives::Network;
use rbitcoin_query::Query;
use rbitcoin_test::mine::{mine_regtest_block, regtest_genesis};
use rbitcoin_test::TestDatadir;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

const RPC_BEARER: &str = "Bearer pass";

fn llvm_cov_wall(default_secs: u64, llvm_secs: u64) -> Duration {
    if std::env::var_os("CARGO_LLVM_COV").is_some() {
        Duration::from_secs(llvm_secs)
    } else {
        Duration::from_secs(default_secs)
    }
}

async fn live_p2p_lock() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

fn reserve_addr() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    addr
}

fn spend_to(prev: Txid, value: Amount, script: Vec<u8>) -> Transaction {
    Transaction {
        version: TxVersion::ONE,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint {
                txid: prev,
                vout: 0,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value,
            script_pubkey: ScriptBuf::from_bytes(script),
        }],
    }
}

/// Coinbases pay script A (`0x51`). One mature spend pays script B (`0x52`).
fn fixture_chain() -> (Vec<bitcoin::Block>, BTreeSet<String>, BTreeSet<String>) {
    let maturity = ChainParams::regtest().coinbase_maturity();
    let genesis = regtest_genesis();
    let mut blocks = vec![genesis.clone()];
    let mut tip = genesis.block_hash();
    let mut time = genesis.header.time;
    let mut script_a = BTreeSet::new();
    let mut script_b = BTreeSet::new();

    let height1 = mine_regtest_block(tip, time + 1, 1, vec![]);
    let matured = height1.txdata[0].compute_txid();
    script_a.insert(matured.to_string());
    tip = height1.block_hash();
    time = height1.header.time;
    blocks.push(height1);

    let last_pad = maturity + 1;
    for height in 2..=last_pad {
        let block = mine_regtest_block(tip, time + 1, height, vec![]);
        script_a.insert(block.txdata[0].compute_txid().to_string());
        tip = block.block_hash();
        time = block.header.time;
        blocks.push(block);
    }

    let spend = spend_to(matured, Amount::from_sat(49_0000_0000), vec![0x52]);
    let spend_txid = spend.compute_txid().to_string();
    let spend_block = mine_regtest_block(tip, time + 1, last_pad + 1, vec![spend]);
    script_a.insert(spend_block.txdata[0].compute_txid().to_string());
    script_a.insert(spend_txid.clone());
    script_b.insert(spend_txid);
    blocks.push(spend_block);
    (blocks, script_a, script_b)
}

async fn start_miner(dir: &Path, addr: SocketAddr) -> P2PNode {
    let query = Query::open_or_create_tiny(dir.join("store")).unwrap();
    P2PNode::start(addr, query, ChainParams::regtest(), Milestone::NONE)
        .await
        .expect("miner listen")
}

fn load_chain(miner: &P2PNode, blocks: &[bitcoin::Block]) {
    for (height, block) in blocks.iter().enumerate() {
        miner
            .ingest_block(height as u32, block.clone())
            .unwrap_or_else(|e| panic!("ingest {height}: {e}"));
    }
}

fn syncer_cfg(dir: &Path, miner: SocketAddr, rpc: SocketAddr, electrum: SocketAddr) -> NodeConfig {
    let mut cfg = NodeConfig::default()
        .with_datadir(dir)
        .with_network(Network::Regtest)
        .with_p2p_listen("127.0.0.1:0".parse().unwrap())
        .with_tiny_heads();
    cfg.listen.connect = vec![NetAddr::Ip(miner)];
    cfg.listen.use_seeds = false;
    cfg.listen.electrum = Some(electrum);
    cfg.shindex = true;
    cfg.rpc.listen = Some(rpc);
    cfg.max_tip_age_secs = Some(u64::MAX);
    std::fs::write(dir.join("rpc.token"), "pass").unwrap();
    cfg
}

fn spawn_run_p2p(cfg: NodeConfig) -> tokio::task::JoinHandle<Result<(), rbitcoin_node::NodeError>> {
    tokio::task::spawn_blocking(move || {
        let _block = rbitcoin_net::BlockingRegion::enter();
        tokio::runtime::Handle::current().block_on(run_p2p(cfg))
    })
}

async fn jsonrpc(addr: SocketAddr, method: &str, params: Value) -> Value {
    let body = json!({"jsonrpc":"1.0","id":"test","method":method,"params":params}).to_string();
    let req = format!(
        "POST / HTTP/1.1\r\nHost: {addr}\r\nAuthorization: {RPC_BEARER}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect(addr).await.expect("rpc connect");
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf);
    let json_body = text.split("\r\n\r\n").nth(1).unwrap_or("").trim();
    serde_json::from_str(json_body)
        .unwrap_or_else(|e| panic!("rpc {method} json: {e} body={json_body}"))
}

async fn history_txids(electrum: SocketAddr, script: &[u8]) -> BTreeSet<String> {
    let mut stream = TcpStream::connect(electrum)
        .await
        .expect("electrum connect");
    let req = json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "blockchain.scripthash.get_history",
        "params": [electrum_scripthash_hex(script)]
    })
    .to_string()
        + "\n";
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).await.unwrap();
    let hist: Value = serde_json::from_str(line.trim())
        .unwrap_or_else(|e| panic!("history json: {e} body={line}"));
    hist["result"]
        .as_array()
        .unwrap_or_else(|| panic!("{hist}"))
        .iter()
        .map(|row| {
            row["tx_hash"]
                .as_str()
                .unwrap_or_else(|| panic!("{row}"))
                .to_string()
        })
        .collect()
}

async fn wait_listeners(addrs: &[SocketAddr]) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let mut missing = None;
        for addr in addrs {
            if TcpStream::connect(*addr).await.is_err() {
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

async fn stop_run_p2p(
    rpc: SocketAddr,
    node: tokio::task::JoinHandle<Result<(), rbitcoin_node::NodeError>>,
) {
    let _ = jsonrpc(rpc, "stop", json!([])).await;
    match tokio::time::timeout(Duration::from_secs(20), node).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(e))) => panic!("run_p2p error after stop: {e}"),
        Ok(Err(e)) => panic!("run_p2p join: {e}"),
        Err(_) => panic!("run_p2p did not exit after stop"),
    }
}

async fn wait_caught_up(
    rpc: SocketAddr,
    electrum: SocketAddr,
    height: u32,
    hash: &str,
    script_a: &BTreeSet<String>,
    script_b: &BTreeSet<String>,
) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let count = jsonrpc(rpc, "getblockcount", json!([])).await;
        let best = jsonrpc(rpc, "getbestblockhash", json!([])).await;
        let height_ok = count["result"].as_u64() == Some(u64::from(height));
        let hash_ok = best["result"].as_str() == Some(hash);
        let hist_a = history_txids(electrum, &[0x51]).await;
        let hist_b = history_txids(electrum, &[0x52]).await;
        if height_ok && hash_ok && &hist_a == script_a && &hist_b == script_b {
            return;
        }
        if Instant::now() >= deadline {
            panic!("catch-up count={count} best={best} A={hist_a:?} B={hist_b:?} want height={height} {hash}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn assert_tip_view(rpc: SocketAddr, height: u32, hash: &str) {
    let info = jsonrpc(rpc, "getblockchaininfo", json!([])).await;
    assert_eq!(
        info["result"]["initialblockdownload"], false,
        "caught up must leave IBD: {info}"
    );
    let tips = jsonrpc(rpc, "getchaintips", json!([])).await;
    let rows = tips["result"]
        .as_array()
        .unwrap_or_else(|| panic!("{tips}"));
    let active = rows
        .iter()
        .find(|tip| tip["status"] == "active")
        .unwrap_or_else(|| panic!("no active tip: {tips}"));
    assert_eq!(active["hash"].as_str(), Some(hash), "{tips}");
    assert_eq!(active["height"].as_u64(), Some(u64::from(height)), "{tips}");
}

fn pack_mark(store: &Path) -> PathBuf {
    let flat = store.join("scripthash.head.packed");
    if flat.is_file() {
        return flat;
    }
    let shard = store.join("scripthash.head").join("00.packed");
    assert!(
        shard.is_file(),
        "pack mark missing under {}",
        store.display()
    );
    shard
}

fn mark_mtime(path: &Path) -> SystemTime {
    std::fs::metadata(path)
        .unwrap_or_else(|e| panic!("metadata {}: {e}", path.display()))
        .modified()
        .unwrap()
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for ent in std::fs::read_dir(src).unwrap() {
        let ent = ent.unwrap();
        let to = dst.join(ent.file_name());
        if ent.file_type().unwrap().is_dir() {
            copy_dir(&ent.path(), &to);
        } else {
            std::fs::copy(ent.path(), &to).unwrap();
        }
    }
}

fn store_tip(store: &Path) -> u32 {
    let query = Query::open_or_create_tiny(store).unwrap();
    query.tip_height().map(|h| h.0).unwrap_or(0)
}

/// Cancel IBD once the tip is in `(0, miner_tip)`. Returns that height.
async fn stop_ibd_below_tip(dir: &Path, miner: SocketAddr, miner_tip: u32) -> u32 {
    let query = Query::open_or_create_tiny(dir.join("store")).unwrap();
    let node = P2PNode::start(
        "127.0.0.1:0".parse().unwrap(),
        query,
        ChainParams::regtest(),
        Milestone::NONE,
    )
    .await
    .expect("partial syncer");
    let hub = Arc::clone(&node.hub);
    let tip = {
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        let peers = [NetAddr::Ip(miner)];
        let sync = node.sync_cancellable(&peers, IbdConfig::for_test(), Some(flag));
        tokio::pin!(sync);
        let mut saw_mid = false;
        let joined = loop {
            tokio::select! {
                result = &mut sync => break result,
                _ = tokio::time::sleep(Duration::from_millis(5)) => {
                    let height = hub.tip_height().unwrap_or(0);
                    if height > 0 && height < miner_tip {
                        saw_mid = true;
                        cancel.store(true, Ordering::SeqCst);
                    }
                }
            }
        };
        joined.expect("cancelled IBD still returns after teardown");
        let tip = hub.tip_height().unwrap_or(0);
        assert!(
            saw_mid && tip > 0 && tip < miner_tip,
            "IBD never observed below the miner tip (saw_mid={saw_mid} tip={tip} miner={miner_tip})"
        );
        tip
    };
    node.shutdown().await;
    tip
}

async fn expect_process_exit_without_electrum(
    node: tokio::task::JoinHandle<Result<(), rbitcoin_node::NodeError>>,
    electrum: SocketAddr,
) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        assert!(
            TcpStream::connect(electrum).await.is_err(),
            "Electrum listened before the chain was complete"
        );
        if node.is_finished() {
            break;
        }
        if Instant::now() >= deadline {
            panic!("syncer with the miner down stayed up");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    match node.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => panic!("incomplete run_p2p: {e}"),
        Err(e) => panic!("incomplete join: {e}"),
    }
}

async fn extend_one(
    miner: &P2PNode,
    rpc: SocketAddr,
    electrum: SocketAddr,
    script_a: &mut BTreeSet<String>,
) -> (u32, String) {
    let before = history_txids(electrum, &[0x51]).await;
    miner
        .hub
        .generate_to_script(1, ScriptBuf::from_bytes(vec![0x51]), vec![])
        .unwrap();
    let height = miner.tip_height().unwrap();
    let hash = miner.hub.tip_hash().unwrap().to_string();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let got = history_txids(electrum, &[0x51]).await;
        let count = jsonrpc(rpc, "getblockcount", json!([])).await;
        let best = jsonrpc(rpc, "getbestblockhash", json!([])).await;
        let added: BTreeSet<_> = got.difference(&before).cloned().collect();
        if count["result"].as_u64() == Some(u64::from(height))
            && best["result"].as_str() == Some(hash.as_str())
            && added.len() == 1
            && got.is_superset(&before)
        {
            script_a.extend(added);
            assert_eq!(&got, script_a);
            return (height, hash);
        }
        if Instant::now() >= deadline {
            panic!("tip-follow height={height} {hash} history={got:?} count={count} best={best}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn end_of_ibd_follow() {
    let _live = live_p2p_lock().await;
    let wall = llvm_cov_wall(90, 180);
    tokio::time::timeout(wall, follow_journey())
        .await
        .expect("end_of_ibd_follow wall");
}

struct Synced {
    height: u32,
    hash: String,
    script_a: BTreeSet<String>,
    script_b: BTreeSet<String>,
}

async fn ibd_then_restart(miner: P2PNode, miner_addr: SocketAddr, synced: &mut Synced) {
    let syncer_dir = TestDatadir::new().unwrap();
    let rpc = reserve_addr();
    let electrum = reserve_addr();
    let node = spawn_run_p2p(syncer_cfg(
        syncer_dir.path().as_path(),
        miner_addr,
        rpc,
        electrum,
    ));
    wait_listeners(&[rpc, electrum]).await;
    wait_caught_up(
        rpc,
        electrum,
        synced.height,
        &synced.hash,
        &synced.script_a,
        &synced.script_b,
    )
    .await;
    assert_tip_view(rpc, synced.height, &synced.hash).await;

    let (height, hash) = extend_one(&miner, rpc, electrum, &mut synced.script_a).await;
    synced.height = height;
    synced.hash = hash;
    assert_tip_view(rpc, synced.height, &synced.hash).await;

    let store = syncer_dir.store_path();
    let mark = pack_mark(&store);
    let packed_at = mark_mtime(&mark);
    assert!(!store.join("scripthash.unsorted").is_dir());
    stop_run_p2p(rpc, node).await;

    let node = spawn_run_p2p(syncer_cfg(
        syncer_dir.path().as_path(),
        miner_addr,
        rpc,
        electrum,
    ));
    wait_listeners(&[rpc, electrum]).await;
    wait_caught_up(
        rpc,
        electrum,
        synced.height,
        &synced.hash,
        &synced.script_a,
        &synced.script_b,
    )
    .await;
    assert_eq!(mark_mtime(&mark), packed_at, "restart collected again");
    assert!(!store.join("scripthash.unsorted").is_dir());
    let (height, hash) = extend_one(&miner, rpc, electrum, &mut synced.script_a).await;
    synced.height = height;
    synced.hash = hash;
    assert_eq!(
        mark_mtime(&mark),
        packed_at,
        "write-behind rewrote the pack mark"
    );
    assert_tip_view(rpc, synced.height, &synced.hash).await;

    miner.shutdown().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_tip_view(rpc, synced.height, &synced.hash).await;
    assert_eq!(history_txids(electrum, &[0x51]).await, synced.script_a);
    assert_eq!(history_txids(electrum, &[0x52]).await, synced.script_b);
    stop_run_p2p(rpc, node).await;
}

async fn finish_partial(dir: &Path, miner: SocketAddr, synced: &Synced) {
    let rpc = reserve_addr();
    let electrum = reserve_addr();
    let node = spawn_run_p2p(syncer_cfg(dir, miner, rpc, electrum));
    wait_listeners(&[rpc, electrum]).await;
    wait_caught_up(
        rpc,
        electrum,
        synced.height,
        &synced.hash,
        &synced.script_a,
        &synced.script_b,
    )
    .await;
    assert_tip_view(rpc, synced.height, &synced.hash).await;
    stop_run_p2p(rpc, node).await;
}

async fn partial_with_miner_down(miner_dir: &Path, miner_addr: SocketAddr, synced: &Synced) {
    let miner = start_miner(miner_dir, miner_addr).await;
    assert_eq!(miner.tip_height().unwrap(), synced.height);
    assert_eq!(miner.hub.tip_hash().unwrap().to_string(), synced.hash);

    let partial_dir = TestDatadir::new().unwrap();
    let partial_tip =
        stop_ibd_below_tip(partial_dir.path().as_path(), miner_addr, synced.height).await;
    let held_dir = TestDatadir::new().unwrap();
    copy_dir(&partial_dir.store_path(), &held_dir.store_path());

    miner.shutdown().await;
    let held_rpc = reserve_addr();
    let held_el = reserve_addr();
    let held = spawn_run_p2p(syncer_cfg(
        held_dir.path().as_path(),
        miner_addr,
        held_rpc,
        held_el,
    ));
    expect_process_exit_without_electrum(held, held_el).await;
    let held_tip = store_tip(&held_dir.store_path());
    assert!(
        held_tip > 0 && held_tip < synced.height,
        "miner-down restart tip={held_tip} partial={partial_tip} full={}",
        synced.height
    );

    let miner = start_miner(miner_dir, miner_addr).await;
    finish_partial(partial_dir.path().as_path(), miner_addr, synced).await;
    finish_partial(held_dir.path().as_path(), miner_addr, synced).await;
    miner.shutdown().await;
}

async fn follow_journey() {
    let (blocks, script_a, script_b) = fixture_chain();
    let miner_dir = TestDatadir::new().unwrap();
    let miner_addr = reserve_addr();
    let miner = start_miner(miner_dir.path().as_path(), miner_addr).await;
    load_chain(&miner, &blocks);
    let mut synced = Synced {
        height: miner.tip_height().unwrap(),
        hash: miner.hub.tip_hash().unwrap().to_string(),
        script_a,
        script_b,
    };
    assert_eq!(synced.height as usize, blocks.len() - 1);

    ibd_then_restart(miner, miner_addr, &mut synced).await;
    partial_with_miner_down(miner_dir.path().as_path(), miner_addr, &synced).await;
}
