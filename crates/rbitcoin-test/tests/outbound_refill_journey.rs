//! Post-catch-up outbound full-relay refill up to `--max-outbound`.
//!
//! Drives `run_p2p` against local listeners. Routable addresses are required
//! in the address book, so the body runs in its own network namespace.

use bitcoin::Block;
use rbitcoin_consensus::{ChainParams, Milestone};
use rbitcoin_net::{AddrMan, NetAddr, P2PNode};
use rbitcoin_node::{run_p2p, NodeConfig, NodeError, P2pListen};
use rbitcoin_primitives::Network;
use rbitcoin_query::Query;
use rbitcoin_test::mine::{mine_regtest_block, regtest_genesis};
use rbitcoin_test::TestDatadir;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::task::JoinHandle;

const MAX_OUT: u32 = 5;
const WANT: usize = 4;

fn llvm_cov_wall(default_secs: u64, llvm_secs: u64) -> Duration {
    if std::env::var_os("CARGO_LLVM_COV").is_some() {
        Duration::from_secs(llvm_secs)
    } else {
        Duration::from_secs(default_secs)
    }
}

fn run_ok(bin: &str, args: &[&str]) {
    let status = Command::new(bin)
        .args(args)
        .status()
        .unwrap_or_else(|e| panic!("{bin} {args:?}: {e}"));
    assert!(status.success(), "{bin} {args:?} -> {status}");
}

fn routable_locals(n: u8) -> Vec<Ipv4Addr> {
    run_ok("ip", &["link", "set", "lo", "up"]);
    let mut ips = Vec::with_capacity(n as usize);
    for i in 1..=n {
        let ip: Ipv4Addr = format!("45.{i}.0.1").parse().expect("ip");
        let cidr = format!("{ip}/32");
        run_ok("ip", &["addr", "add", &cidr, "dev", "lo"]);
        ips.push(ip);
    }
    ips
}

fn two_blocks() -> Vec<Block> {
    let genesis = regtest_genesis();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_secs() as u32;
    let first = mine_regtest_block(genesis.block_hash(), now, 1, vec![]);
    let second = mine_regtest_block(first.block_hash(), now.saturating_add(1), 2, vec![]);
    vec![first, second]
}

async fn listen_with_chain(dir: &Path, ip: Ipv4Addr, blocks: &[Block]) -> P2PNode {
    let query =
        Query::open_or_create_tiny(dir.join(format!("listener-{ip}"))).expect("listener store");
    let node = P2PNode::start(
        SocketAddr::from((ip, 0)),
        query,
        ChainParams::regtest(),
        Milestone::NONE,
        rbitcoin_net::REDIAL_INTERVAL_TEST,
    )
    .await
    .unwrap_or_else(|e| panic!("listen {ip}: {e}"));
    for (h, block) in blocks.iter().enumerate() {
        node.ingest_block(block.clone())
            .unwrap_or_else(|e| panic!("ingest {ip} block {}: {e}", h + 1));
    }
    node
}

fn write_book(dir: &Path, addrs: &[SocketAddr]) {
    let mut book = AddrMan::new();
    for addr in addrs {
        book.add(*addr);
    }
    assert_eq!(book.len(), addrs.len(), "routable book rows");
    book.save(&dir.join("peers")).expect("peers file");
}

fn token(dir: &Path) {
    std::fs::create_dir_all(dir).expect("datadir");
    let path = dir.join("rpc.token");
    std::fs::write(&path, "pass").expect("rpc token");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("token mode");
    }
}

fn base_cfg(dir: &Path) -> NodeConfig {
    token(dir);
    let mut cfg = NodeConfig::default()
        .with_datadir(dir)
        .with_network(Network::Regtest)
        .with_tiny_heads();
    cfg.listen.p2p = P2pListen::Off;
    cfg.listen.p2p_explicit = true;
    cfg.listen.use_seeds = false;
    cfg.listen.discover = false;
    cfg.listen.max_inbound = 0;
    cfg.listen.max_outbound = MAX_OUT;
    cfg.rpc.listen = Some("127.0.0.1:0".parse().expect("rpc"));
    cfg.max_tip_age_secs = Some(u64::MAX);
    cfg.connect_redial = rbitcoin_net::REDIAL_INTERVAL_TEST;
    cfg
}

fn published(dir: &Path, name: &str) -> PathBuf {
    dir.join("run").join(format!("{name}.addr"))
}

async fn published_listener(dir: &Path, name: &str, wait: Duration) -> SocketAddr {
    let path = published(dir, name);
    let deadline = Instant::now() + wait;
    loop {
        if let Some(addr) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| text.trim().parse().ok())
        {
            return addr;
        }
        if Instant::now() >= deadline {
            panic!("{name} listener not up at {}", path.display());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn spawn_run(cfg: NodeConfig) -> JoinHandle<Result<(), NodeError>> {
    match std::fs::remove_dir_all(cfg.datadir.path().join("run")) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => panic!("clear published listeners: {e}"),
    }
    tokio::task::spawn_blocking(move || {
        let _block = rbitcoin_net::BlockingRegion::enter();
        tokio::runtime::Handle::current().block_on(run_p2p(cfg))
    })
}

async fn peer_rows(rpc: SocketAddr) -> Vec<Value> {
    let v = rbitcoin_test::jsonrpc::try_jsonrpc(rpc, "getpeerinfo", json!([])).await;
    let Ok(v) = v else {
        return Vec::new();
    };
    v["result"].as_array().cloned().unwrap_or_default()
}

fn conn_type(row: &Value) -> &str {
    row["connection_type"].as_str().unwrap_or("")
}

fn full_relay(rows: &[Value]) -> Vec<&Value> {
    rows.iter()
        .filter(|row| conn_type(row) == "outbound-full-relay")
        .collect()
}

fn type_list(rows: &[Value]) -> String {
    rows.iter().map(conn_type).collect::<Vec<_>>().join(",")
}

fn ids_of(rows: &[&Value]) -> BTreeSet<u64> {
    rows.iter().filter_map(|row| row["id"].as_u64()).collect()
}

/// Count of live `outbound-full-relay` rows, or a panic once that count
/// has stopped climbing short of `want`.
async fn wait_full_relay(rpc: SocketAddr, want: usize, max_out: usize) -> Vec<Value> {
    let start = Instant::now();
    let deadline = start + Duration::from_secs(25);
    let mut last = usize::MAX;
    let mut stable = Instant::now();
    loop {
        let rows = peer_rows(rpc).await;
        let n = full_relay(&rows).len();
        assert!(
            n <= max_out,
            "outbound-full-relay {n} exceeded --max-outbound {max_out}: {}",
            type_list(&rows)
        );
        if n >= want {
            assert!(full_relay(&rows)
                .iter()
                .all(|row| conn_type(row) == "outbound-full-relay"));
            return rows;
        }
        if n == last
            && stable.elapsed() >= Duration::from_secs(2)
            && start.elapsed() >= Duration::from_secs(2)
        {
            panic!(
                "outbound-full-relay settled at {n}, want at least {want} (max-outbound {max_out}); types=[{}]",
                type_list(&rows)
            );
        }
        if n != last {
            last = n;
            stable = Instant::now();
        }
        if Instant::now() >= deadline {
            panic!(
                "outbound-full-relay still {n} after {}s, want at least {want}; types=[{}]",
                start.elapsed().as_secs(),
                type_list(&rows)
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn stop_node(rpc: SocketAddr, join: JoinHandle<Result<(), NodeError>>) {
    let _ = rbitcoin_test::jsonrpc::try_jsonrpc(rpc, "stop", json!([])).await;
    let joined = tokio::time::timeout(Duration::from_secs(30), join).await;
    match joined {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(e))) => panic!("run_p2p: {e}"),
        Ok(Err(e)) => panic!("run_p2p join: {e}"),
        Err(_) => panic!("run_p2p did not stop"),
    }
}

async fn book_refill(root: &Path, listeners: &[P2PNode]) {
    let dir = root.join("book");
    let addrs: Vec<SocketAddr> = listeners.iter().map(|n| n.local_addr).collect();
    write_book(&dir, &addrs);
    let cfg = base_cfg(&dir);
    let join = spawn_run(cfg);
    let rpc = published_listener(&dir, "rpc", Duration::from_secs(40)).await;
    let rows = wait_full_relay(rpc, WANT, MAX_OUT as usize).await;
    let before = ids_of(&full_relay(&rows));
    assert!(
        before.len() >= WANT,
        "first post-catch-up dial must exceed 3 outbound-full-relay, got {}",
        type_list(&rows)
    );
    for id in &before {
        let _ = rbitcoin_test::jsonrpc::try_jsonrpc(rpc, "disconnectnode", json!(["", id])).await;
    }
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let rows = peer_rows(rpc).await;
        let live = full_relay(&rows);
        let ids = ids_of(&live);
        let n = live.len();
        assert!(
            n <= MAX_OUT as usize,
            "refill grew past --max-outbound: {}",
            type_list(&rows)
        );
        let old_gone = before.iter().all(|id| !ids.contains(id));
        if n >= WANT && old_gone {
            break;
        }
        if Instant::now() >= deadline {
            panic!(
                "after disconnect outbound-full-relay={n} (want at least {WANT} with the old ids gone) without a stale-tip wait; types=[{}]",
                type_list(&rows)
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    stop_node(rpc, join).await;
}

async fn learned_book(root: &Path, seed: &P2PNode, others: &[SocketAddr]) {
    let mut book = AddrMan::new();
    for addr in others {
        book.add(*addr);
    }
    assert!(
        book.len() >= WANT,
        "seed must advertise the other listeners"
    );
    seed.peers
        .set_addrman(std::sync::Arc::new(std::sync::Mutex::new(book)));
    let dir = root.join("fetch");
    let mut cfg = base_cfg(&dir);
    cfg.listen.seednodes = vec![seed.local_addr.to_string()];
    let join = spawn_run(cfg);
    let rpc = published_listener(&dir, "rpc", Duration::from_secs(40)).await;
    // getaddr returns 23% of the book (at least one). Those learned rows are
    // dialed as outbound-full-relay; the seed session itself stays addr-fetch.
    let rows = wait_learned_relay(rpc, others, MAX_OUT as usize).await;
    let want: std::collections::BTreeSet<String> =
        others.iter().map(|addr| addr.to_string()).collect();
    let learned = full_relay(&rows)
        .iter()
        .filter(|row| want.contains(row["addr"].as_str().unwrap_or("")))
        .count();
    assert!(
        learned >= 1,
        "addresses learned into an empty book were not dialed as outbound-full-relay: {}",
        type_list(&rows)
    );
    stop_node(rpc, join).await;
}

async fn wait_learned_relay(rpc: SocketAddr, others: &[SocketAddr], max_out: usize) -> Vec<Value> {
    let want: std::collections::BTreeSet<String> =
        others.iter().map(|addr| addr.to_string()).collect();
    let start = Instant::now();
    let deadline = start + Duration::from_secs(25);
    let mut last = usize::MAX;
    let mut stable = Instant::now();
    loop {
        let rows = peer_rows(rpc).await;
        let live = full_relay(&rows);
        assert!(
            live.len() <= max_out,
            "outbound-full-relay {} exceeded --max-outbound {max_out}: {}",
            live.len(),
            type_list(&rows)
        );
        for row in &live {
            let addr = row["addr"].as_str().unwrap_or("");
            assert!(
                want.contains(addr),
                "learned dial left the seed book: {addr} types=[{}]",
                type_list(&rows)
            );
        }
        let n = live.len();
        if n >= 1 {
            return rows;
        }
        if n == last
            && stable.elapsed() >= Duration::from_secs(2)
            && start.elapsed() >= Duration::from_secs(2)
        {
            panic!(
                "outbound-full-relay settled at {n} after an addr-fetch reply; types=[{}]",
                type_list(&rows)
            );
        }
        if n != last {
            last = n;
            stable = Instant::now();
        }
        if Instant::now() >= deadline {
            panic!(
                "no outbound-full-relay from the learned book after {}s; types=[{}]",
                start.elapsed().as_secs(),
                type_list(&rows)
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn assert_no_book_peers(rpc: SocketAddr, book: &[SocketAddr], label: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut saw_manual = false;
    let mut manual_at = None;
    loop {
        let rows = peer_rows(rpc).await;
        if !full_relay(&rows).is_empty() {
            panic!(
                "{label} gained outbound-full-relay from the book: {}",
                type_list(&rows)
            );
        }
        for row in &rows {
            let addr = row["addr"].as_str().unwrap_or("");
            if book.iter().any(|b| b.to_string() == addr) {
                panic!("{label} dialed book address {addr}: {}", type_list(&rows));
            }
            if conn_type(row) == "manual" {
                saw_manual = true;
            }
        }
        if saw_manual && manual_at.is_none() {
            manual_at = Some(Instant::now());
        }
        if manual_at.is_some_and(|t| t.elapsed() >= Duration::from_secs(2)) {
            return;
        }
        if Instant::now() >= deadline {
            panic!(
                "{label} did not keep a manual session clear of the book; types=[{}]",
                type_list(&rows)
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn pinned_connect(root: &Path, pinned: SocketAddr, book: &[SocketAddr]) {
    let dir = root.join("connect");
    write_book(&dir, book);
    let mut cfg = base_cfg(&dir);
    cfg.listen.connect = vec![NetAddr::Ip(pinned)];
    let join = spawn_run(cfg);
    let rpc = published_listener(&dir, "rpc", Duration::from_secs(40)).await;
    assert_no_book_peers(rpc, book, "--connect").await;
    stop_node(rpc, join).await;
}

async fn pinned_trust(root: &Path, pinned: SocketAddr, book: &[SocketAddr]) {
    let dir = root.join("trust");
    write_book(&dir, book);
    let mut cfg = base_cfg(&dir);
    cfg.trust_peer = Some(NetAddr::Ip(pinned));
    cfg.seal_trust_peer().expect("trust-peer");
    cfg.connect_redial = rbitcoin_net::REDIAL_INTERVAL_TEST;
    cfg.rpc.listen = Some("127.0.0.1:0".parse().expect("rpc"));
    cfg.max_tip_age_secs = Some(u64::MAX);
    let join = spawn_run(cfg);
    let rpc = published_listener(&dir, "rpc", Duration::from_secs(40)).await;
    assert_no_book_peers(rpc, book, "--trust-peer").await;
    stop_node(rpc, join).await;
}

async fn journey() {
    let ips = routable_locals(5);
    let home = TestDatadir::new().expect("temp");
    let root = home.path();
    let blocks = two_blocks();
    let mut listeners = Vec::new();
    for ip in ips {
        listeners.push(listen_with_chain(&root, ip, &blocks).await);
    }
    book_refill(&root, &listeners).await;
    let others: Vec<SocketAddr> = listeners.iter().skip(1).map(|n| n.local_addr).collect();
    learned_book(&root, &listeners[0], &others).await;
    let pinned = listeners[0].local_addr;
    let book: Vec<SocketAddr> = listeners.iter().skip(1).map(|n| n.local_addr).collect();
    pinned_connect(&root, pinned, &book).await;
    pinned_trust(&root, pinned, &book).await;
    for node in listeners {
        node.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn outbound_full_relay_refills_to_max_outbound() {
    if std::env::var_os("RBITCOIN_REFILL_NS").is_none() {
        let exe = std::env::current_exe().expect("test executable");
        let status = Command::new("unshare")
            .args(["--net", "--user", "--map-root-user", "--"])
            .arg(&exe)
            .arg("--exact")
            .arg("outbound_refill_journey::outbound_full_relay_refills_to_max_outbound")
            .arg("--test-threads=1")
            .env("RBITCOIN_REFILL_NS", "1")
            .status()
            .expect("unshare");
        assert!(status.success(), "refill journey failed in netns: {status}");
        return;
    }
    let wall = llvm_cov_wall(60, 120);
    tokio::time::timeout(wall, journey())
        .await
        .expect("outbound_full_relay_refills_to_max_outbound wall");
}
