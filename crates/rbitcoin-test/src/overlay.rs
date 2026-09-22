//! Helpers for the labeled overlay-functional journeys (real private meshes).

use bitcoin::hashes::{sha256, Hash};
use rbitcoin_node::NodeConfig;
use rbitcoin_primitives::Network;
use serde_json::Value;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::task::JoinHandle;

pub const RPC_BEARER: &str = "Bearer pass";

pub struct OverlayEnv {
    pub tor_socks: SocketAddr,
    pub tor_control: SocketAddr,
    pub tor_cookie: PathBuf,
    pub i2p_sam: SocketAddr,
    pub i2p_sam_b: SocketAddr,
    pub cjdns_a: std::net::Ipv6Addr,
    pub cjdns_b: std::net::Ipv6Addr,
}

pub fn require_env() -> OverlayEnv {
    OverlayEnv {
        tor_socks: req_addr("OVERLAY_TOR_SOCKS"),
        tor_control: req_addr("OVERLAY_TOR_CONTROL"),
        tor_cookie: PathBuf::from(req("OVERLAY_TOR_COOKIE")),
        i2p_sam: req_addr("OVERLAY_I2P_SAM"),
        i2p_sam_b: req_addr("OVERLAY_I2P_SAM_B"),
        cjdns_a: req("OVERLAY_CJDNS_A")
            .parse()
            .unwrap_or_else(|e| panic!("OVERLAY_CJDNS_A: {e}")),
        cjdns_b: req("OVERLAY_CJDNS_B")
            .parse()
            .unwrap_or_else(|e| panic!("OVERLAY_CJDNS_B: {e}")),
    }
}

fn req(name: &str) -> String {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("{name} is unset; run via ./scripts/overlay-functional/run.sh"))
}

fn req_addr(name: &str) -> SocketAddr {
    req(name).parse().unwrap_or_else(|e| panic!("{name}: {e}"))
}

pub async fn live_p2p_lock() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

pub fn ephemeral_addr() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    addr
}

pub fn write_rpc_token(datadir: &Path) {
    std::fs::write(datadir.join("rpc.token"), "pass").unwrap();
}

pub fn base_node(datadir: &Path) -> NodeConfig {
    let mut cfg = NodeConfig::default()
        .with_datadir(datadir)
        .with_network(Network::Regtest)
        .with_tiny_heads();
    cfg.listen.use_seeds = false;
    cfg.listen.discover = false;
    cfg.max_tip_age_secs = Some(u64::MAX);
    cfg
}

pub struct NodeProc {
    rpc: SocketAddr,
    handle: Option<JoinHandle<Result<(), rbitcoin_node::NodeError>>>,
}

impl NodeProc {
    pub async fn stop(mut self) {
        let _ = jsonrpc(self.rpc, "stop", serde_json::json!([])).await;
        let handle = self.handle.take().expect("stop once");
        match tokio::time::timeout(Duration::from_secs(15), handle).await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(e))) => panic!("run_p2p: {e}"),
            Ok(Err(e)) => panic!("run_p2p join: {e}"),
            Err(_) => panic!("run_p2p did not exit after stop"),
        }
    }
}

impl Drop for NodeProc {
    fn drop(&mut self) {
        if let Some(h) = self.handle.take() {
            h.abort();
        }
    }
}

pub async fn spawn_node(cfg: NodeConfig, rpc: SocketAddr) -> NodeProc {
    // Dedicated blocking pool thread: `run_p2p` does tip-accept on the
    // future that `block_on` polls. Tokio names spawn_blocking threads
    // `tokio-rt-worker`; BlockingRegion is the same seam as live P2P tests.
    let handle = tokio::task::spawn_blocking(move || {
        let _block = rbitcoin_net::BlockingRegion::enter();
        tokio::runtime::Handle::current().block_on(rbitcoin_node::run_p2p(cfg))
    });
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if handle.is_finished() {
            match handle.await {
                Ok(Ok(())) => panic!("run_p2p exited before rpc {rpc}"),
                Ok(Err(e)) => panic!("run_p2p: {e}"),
                Err(e) => panic!("run_p2p join: {e}"),
            }
        }
        if TcpStream::connect(rpc).await.is_ok() {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if handle.is_finished() {
                match handle.await {
                    Ok(Ok(())) => panic!("run_p2p exited after rpc {rpc}"),
                    Ok(Err(e)) => panic!("run_p2p: {e}"),
                    Err(e) => panic!("run_p2p join: {e}"),
                }
            }
            return NodeProc {
                rpc,
                handle: Some(handle),
            };
        }
        if Instant::now() >= deadline {
            panic!("listeners not up ({rpc})");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub async fn jsonrpc(addr: SocketAddr, method: &str, params: Value) -> Value {
    jsonrpc_try(addr, method, params)
        .await
        .unwrap_or_else(|e| panic!("{e}"))
}

async fn jsonrpc_try(addr: SocketAddr, method: &str, params: Value) -> Result<Value, String> {
    let body = serde_json::json!({"jsonrpc":"1.0","id":"test","method":method,"params":params})
        .to_string();
    let req = format!(
        "POST / HTTP/1.1\r\nHost: {addr}\r\nAuthorization: {RPC_BEARER}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect(addr)
        .await
        .map_err(|e| format!("rpc connect: {e}"))?;
    stream
        .write_all(req.as_bytes())
        .await
        .map_err(|e| format!("rpc write: {e}"))?;
    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .await
        .map_err(|e| format!("rpc read: {e}"))?;
    let text = String::from_utf8_lossy(&buf);
    let json_body = text.split("\r\n\r\n").nth(1).unwrap_or("").trim();
    serde_json::from_str(json_body).map_err(|e| format!("rpc {method} json: {e} body={json_body}"))
}

pub async fn wait_jsonrpc(
    addr: SocketAddr,
    method: &str,
    params: Value,
    wall: Duration,
    ok: impl Fn(&Value) -> bool,
) -> Value {
    let deadline = Instant::now() + wall;
    loop {
        let last = match jsonrpc_try(addr, method, params.clone()).await {
            Ok(v) if ok(&v) => return v,
            Ok(v) => v.to_string(),
            Err(e) => e,
        };
        if Instant::now() >= deadline {
            panic!("{method} wall {wall:?}: {last}");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

pub fn localaddresses(info: &Value) -> &[Value] {
    info["result"]["localaddresses"]
        .as_array()
        .expect("localaddresses")
}

pub fn onion_host(row: &Value) -> Option<&str> {
    row["address"].as_str().filter(|s| s.ends_with(".onion"))
}

pub async fn wait_p2p_onion(rpc: SocketAddr, wall: Duration) -> (String, u16) {
    let info = wait_jsonrpc(rpc, "getnetworkinfo", serde_json::json!([]), wall, |v| {
        localaddresses(v)
            .iter()
            .any(|r| onion_host(r).is_some() && r["port"] == Network::Regtest.default_p2p_port())
    })
    .await;
    for row in localaddresses(&info) {
        if let Some(host) = onion_host(row) {
            if row["port"] == Network::Regtest.default_p2p_port() {
                return (host.to_string(), Network::Regtest.default_p2p_port());
            }
        }
    }
    panic!("p2p onion missing: {info}");
}

/// SOCKS domain CONNECT until the onion TCP port accepts (HSDir / ADD_ONION ready).
pub async fn wait_socks_onion(proxy: SocketAddr, host: &str, port: u16, wall: Duration) {
    let deadline = Instant::now() + wall;
    loop {
        let dialer = rbitcoin_net::Dialer::socks(proxy, true);
        match dialer.connect_domain(host, port).await {
            Ok(_) => return,
            Err(e) => {
                if Instant::now() >= deadline {
                    panic!("SOCKS onion {host}:{port} wall {wall:?}: {e}");
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// SAM STREAM CONNECT until the dest accepts (leaseset published on the private mesh).
/// One throwaway session, then drop it before the product node uses this SAM.
pub async fn wait_i2p_stream(sam: SocketAddr, dest_b32: &str, wall: Duration) {
    let deadline = Instant::now() + wall;
    let session = loop {
        match rbitcoin_net::I2pSam::connect(sam).await {
            Ok(s) => break s,
            Err(e) => {
                if Instant::now() >= deadline {
                    panic!("I2P session {sam} wall {wall:?}: {e}");
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    loop {
        match session.stream_connect(dest_b32).await {
            Ok(_) => return,
            Err(e) => {
                if Instant::now() >= deadline {
                    panic!("I2P STREAM {dest_b32} wall {wall:?}: {e}");
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

pub async fn wait_onion_port(rpc: SocketAddr, port: u16, wall: Duration) -> String {
    let info = wait_jsonrpc(rpc, "getnetworkinfo", serde_json::json!([]), wall, |v| {
        localaddresses(v)
            .iter()
            .any(|r| onion_host(r).is_some() && r["port"] == port)
    })
    .await;
    for row in localaddresses(&info) {
        if onion_host(row).is_some() && row["port"] == port {
            return onion_host(row).unwrap().to_string();
        }
    }
    panic!("onion port {port} missing: {info}");
}

pub async fn wait_peer_network(rpc: SocketAddr, net: &str, inbound: bool, wall: Duration) -> Value {
    wait_jsonrpc(rpc, "getpeerinfo", serde_json::json!([]), wall, |v| {
        v["result"].as_array().is_some_and(|rows| {
            rows.iter().any(|p| {
                p["network"] == net && p["inbound"] == inbound && p["startingheight"] != -1
            })
        })
    })
    .await
}

/// I2P b32 hostname from a SAM `DESTINATION=` / `{datadir}/i2p/p2p.priv` blob.
/// SESSION STATUS returns the private dest; hash is of the public Identity prefix.
pub fn i2p_b32_from_dest(dest: &str) -> String {
    let raw = decode_i2p_b64(dest.trim());
    let digest = sha256::Hash::hash(i2p_public_ident(&raw));
    format!("{}.b32.i2p", encode_b32_nopad(digest.as_byte_array()))
}

fn i2p_public_ident(raw: &[u8]) -> &[u8] {
    if raw.len() < 387 {
        return raw;
    }
    let cert_len = u16::from_be_bytes([raw[385], raw[386]]) as usize;
    let ident_len = 387usize.saturating_add(cert_len);
    if ident_len <= raw.len() {
        &raw[..ident_len]
    } else {
        raw
    }
}

fn decode_i2p_b64(s: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut acc = 0u32;
    let mut n = 0;
    for c in s.chars() {
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '-' => 62,
            '~' => 63,
            '+' => 62,
            '/' => 63,
            '=' => break,
            _ => continue,
        };
        acc = (acc << 6) | v;
        n += 6;
        if n >= 8 {
            n -= 8;
            bytes.push((acc >> n) as u8);
        }
    }
    bytes
}

fn encode_b32_nopad(data: &[u8]) -> String {
    const ALPH: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut out = String::new();
    let mut acc = 0u32;
    let mut n = 0;
    for &b in data {
        acc = (acc << 8) | u32::from(b);
        n += 8;
        while n >= 5 {
            n -= 5;
            out.push(ALPH[((acc >> n) & 31) as usize] as char);
        }
    }
    if n > 0 {
        out.push(ALPH[((acc << (5 - n)) & 31) as usize] as char);
    }
    out
}

pub async fn socks_http_get(proxy: SocketAddr, host: &str, port: u16, path: &str) -> (u16, String) {
    let dialer = rbitcoin_net::Dialer::socks(proxy, true);
    let mut stream = dialer
        .connect_domain(host, port)
        .await
        .unwrap_or_else(|e| panic!("SOCKS {host}:{port}: {e}"));
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf);
    let status = text
        .split_whitespace()
        .nth(1)
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

pub async fn socks_electrum_rpc(
    proxy: SocketAddr,
    host: &str,
    port: u16,
    method: &str,
    params: Value,
) -> Value {
    let dialer = rbitcoin_net::Dialer::socks(proxy, true);
    let mut stream = dialer
        .connect_domain(host, port)
        .await
        .unwrap_or_else(|e| panic!("SOCKS electrum {host}:{port}: {e}"));
    let req = serde_json::json!({"id":1,"jsonrpc":"2.0","method":method,"params":params})
        .to_string()
        + "\n";
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .await
        .expect("electrum read");
    serde_json::from_str(line.trim())
        .unwrap_or_else(|e| panic!("electrum {method}: {e} body={line}"))
}

pub async fn wait_file(path: &Path, wall: Duration) -> String {
    let deadline = Instant::now() + wall;
    loop {
        if let Ok(s) = std::fs::read_to_string(path) {
            let t = s.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
        if Instant::now() >= deadline {
            panic!("missing {} after {wall:?}", path.display());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::hashes::{sha256, Hash};

    fn i2p_b64_encode(raw: &[u8]) -> String {
        const ALPH: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-~";
        let mut out = String::new();
        let mut i = 0;
        while i < raw.len() {
            let b0 = raw[i];
            let b1 = if i + 1 < raw.len() { raw[i + 1] } else { 0 };
            let b2 = if i + 2 < raw.len() { raw[i + 2] } else { 0 };
            out.push(ALPH[(b0 >> 2) as usize] as char);
            out.push(ALPH[(((b0 & 3) << 4) | (b1 >> 4)) as usize] as char);
            if i + 1 < raw.len() {
                out.push(ALPH[(((b1 & 15) << 2) | (b2 >> 6)) as usize] as char);
            }
            if i + 2 < raw.len() {
                out.push(ALPH[(b2 & 63) as usize] as char);
            }
            i += 3;
        }
        out
    }

    #[test]
    fn i2p_b32_hashes_identity_prefix() {
        let mut ident = vec![7u8; 387];
        ident[384] = 0;
        ident[385] = 0;
        ident[386] = 0;
        let mut privd = ident.clone();
        privd.extend_from_slice(&[9u8; 80]);
        let a = i2p_b32_from_dest(&i2p_b64_encode(&ident));
        let b = i2p_b32_from_dest(&i2p_b64_encode(&privd));
        assert_eq!(a, b, "private dest must hash the Identity prefix");
        let full = format!(
            "{}.b32.i2p",
            encode_b32_nopad(sha256::Hash::hash(&privd).as_byte_array())
        );
        assert_ne!(a, full);
        assert!(a.ends_with(".b32.i2p"), "{a}");
        assert_eq!(a.len(), 52 + ".b32.i2p".len(), "{a}");
    }
}
