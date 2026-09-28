//! Health listener (`--health-listen`): unauthenticated `GET /healthz` and
//! `GET /readyz` for process probes, and `GET /metrics` with `--metrics`.
//!
//! It binds at the top of [`crate::run_p2p`], before the store opens, so it
//! answers through a schema migration, catch-up, and index materialize. The
//! RPC, Electrum, and Esplora listeners bind only after those.

mod metrics;

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use rbitcoin_log::{info, warn};
use rbitcoin_net::{BlockingRegion, ChainHub, MempoolHub, PeerHub};
use rbitcoin_primitives::Network;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;

/// Probes in flight at once. Probers send one request per period.
const MAX_CONCURRENT: usize = 16;
/// Per-request wall. Probe timeouts are usually 1s.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// Tip and scripthash index lag that `/readyz` still calls ready.
pub(crate) const READY_LAG_BLOCKS: u32 = 6;

/// Where [`crate::run_p2p`] is in bring-up. `/readyz` is 503 until `Following`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Store open: schema migration, backfill, spend replay.
    Opening,
    /// P2P, mempool, and proxy bring-up before catch-up.
    Starting,
    /// Initial block download.
    CatchUp,
    /// Tip-mode entry (scripthash materialize), follow peers, listeners.
    Indexing,
    /// Tip-follow loop.
    Following,
    /// Shutdown flush.
    Stopping,
}

impl Phase {
    const ALL: [Self; 6] = [
        Self::Opening,
        Self::Starting,
        Self::CatchUp,
        Self::Indexing,
        Self::Following,
        Self::Stopping,
    ];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Opening => "opening",
            Self::Starting => "starting",
            Self::CatchUp => "catch-up",
            Self::Indexing => "indexing",
            Self::Following => "following",
            Self::Stopping => "stopping",
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Opening,
            1 => Self::Starting,
            2 => Self::CatchUp,
            3 => Self::Indexing,
            4 => Self::Following,
            _ => Self::Stopping,
        }
    }
}

/// Bring-up state the health routes read. `run_p2p` writes the phase at each
/// transition; every read is an atomic load or a `OnceLock` get.
pub(crate) struct NodeStatus {
    phase: AtomicU8,
    network: Network,
    sh_index: bool,
    started: SystemTime,
    chain: OnceLock<Arc<ChainHub>>,
    peers: OnceLock<Arc<PeerHub>>,
    mempool: OnceLock<Arc<MempoolHub>>,
    /// Configured listeners that failed to bind (RPC, Electrum, Esplora only warn).
    unbound: OnceLock<Vec<&'static str>>,
}

impl NodeStatus {
    pub(crate) fn new(network: Network, sh_index: bool) -> Arc<Self> {
        Arc::new(Self {
            phase: AtomicU8::new(Phase::Opening as u8),
            network,
            sh_index,
            started: SystemTime::now(),
            chain: OnceLock::new(),
            peers: OnceLock::new(),
            mempool: OnceLock::new(),
            unbound: OnceLock::new(),
        })
    }

    pub(crate) fn enter(&self, phase: Phase) {
        self.phase.store(phase as u8, Ordering::Release);
    }

    pub(crate) fn attach_p2p(&self, chain: &Arc<ChainHub>, peers: &Arc<PeerHub>) {
        let _ = self.chain.set(Arc::clone(chain));
        let _ = self.peers.set(Arc::clone(peers));
    }

    pub(crate) fn attach_mempool(&self, mempool: &Arc<MempoolHub>) {
        let _ = self.mempool.set(Arc::clone(mempool));
    }

    /// Enter [`Phase::Following`] with the configured listeners that did not bind.
    pub(crate) fn follow(&self, unbound: Vec<&'static str>) {
        let _ = self.unbound.set(unbound);
        self.enter(Phase::Following);
    }

    fn phase(&self) -> Phase {
        Phase::from_u8(self.phase.load(Ordering::Acquire))
    }

    /// Chain reads may touch the store; call from the blocking pool.
    fn ready_snapshot(&self) -> ReadySnapshot {
        let phase = self.phase();
        let mut snap = ReadySnapshot {
            phase,
            unbound: self.unbound.get().cloned().unwrap_or_default(),
            in_ibd: false,
            blocks: 0,
            headers: 0,
            tip_age_secs: None,
            max_tip_age_secs: 0,
            sh_lag: None,
        };
        if phase != Phase::Following {
            return snap;
        }
        if let Some(chain) = self.chain.get() {
            snap.in_ibd = chain.in_ibd();
            snap.blocks = chain.query.tip_height().map_or(0, |h| h.0);
            snap.headers = chain.best_header_height();
            snap.tip_age_secs = chain
                .tip_header()
                .map(|h| chain.clock.now_secs().saturating_sub(u64::from(h.time)));
            snap.max_tip_age_secs = chain.max_tip_age_secs();
            snap.sh_lag = self.sh_index.then(|| chain.query.sh_lag_heights());
        }
        snap
    }
}

/// The inputs of one `/readyz` answer.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ReadySnapshot {
    phase: Phase,
    unbound: Vec<&'static str>,
    /// Same value as RPC `getblockchaininfo.initialblockdownload`.
    in_ibd: bool,
    blocks: u32,
    headers: u32,
    /// Seconds since the tip block time; `None` with no tip. `in_ibd` latches
    /// off after the first exit — this one keeps reporting a stale tip.
    tip_age_secs: Option<u64>,
    max_tip_age_secs: u64,
    /// `None` without `--sh-index`.
    sh_lag: Option<u32>,
}

/// `Ok` when ready; otherwise the first failing gate, in check order.
fn readiness(s: &ReadySnapshot) -> Result<(), String> {
    if s.phase != Phase::Following {
        return Err(s.phase.as_str().to_string());
    }
    if !s.unbound.is_empty() {
        return Err(format!("{} not listening", s.unbound.join(", ")));
    }
    if s.in_ibd {
        return Err("initial block download".into());
    }
    if let Some(age) = s.tip_age_secs {
        if age > s.max_tip_age_secs {
            return Err(format!("tip stale (last block {age}s ago)"));
        }
    }
    let behind = s.headers.saturating_sub(s.blocks);
    if behind > READY_LAG_BLOCKS {
        return Err(format!("tip {behind} blocks behind headers"));
    }
    match s.sh_lag {
        Some(lag) if lag > READY_LAG_BLOCKS => {
            Err(format!("scripthash index {lag} blocks behind tip"))
        }
        _ => Ok(()),
    }
}

/// Bound health listener. Dropping it stops serving.
pub(crate) struct HealthHandle {
    task: JoinHandle<()>,
}

impl Drop for HealthHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Bind `addr` and serve the health routes (and `/metrics` when `metrics`)
/// until the handle drops.
pub(crate) async fn run_health(
    addr: SocketAddr,
    status: Arc<NodeStatus>,
    metrics: bool,
) -> std::io::Result<HealthHandle> {
    let listener = TcpListener::bind(addr).await?;
    let local_addr = listener.local_addr()?;
    if !local_addr.ip().is_loopback() {
        warn!("health: {local_addr} is not loopback; /healthz and /readyz are unauthenticated");
    }
    let app = router(status, metrics);
    let task = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            warn!("health: serve ended: {e}");
        }
    });
    info!("health HTTP on {local_addr}");
    Ok(HealthHandle { task })
}

fn router(status: Arc<NodeStatus>, metrics: bool) -> Router {
    let routes = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz));
    let routes = if metrics {
        routes.route("/metrics", get(scrape))
    } else {
        routes
    };
    routes
        // Outer → inner: concurrency → body (GET only, so none) → timeout.
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(RequestBodyLimitLayer::new(0))
        .layer(ConcurrencyLimitLayer::new(MAX_CONCURRENT))
        .with_state(status)
}

async fn healthz() -> &'static str {
    "ok\n"
}

async fn scrape(State(status): State<Arc<NodeStatus>>) -> Response {
    let body = tokio::task::spawn_blocking(move || {
        let _g = BlockingRegion::enter();
        metrics::render(&status)
    })
    .await;
    match body {
        Ok(body) => ([(header::CONTENT_TYPE, metrics::CONTENT_TYPE)], body).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("metrics: {e}\n")).into_response(),
    }
}

async fn readyz(State(status): State<Arc<NodeStatus>>) -> (StatusCode, String) {
    let snap = tokio::task::spawn_blocking(move || {
        let _g = BlockingRegion::enter();
        status.ready_snapshot()
    })
    .await;
    match snap.as_ref().map(readiness) {
        Ok(Ok(())) => (StatusCode::OK, "ok\n".into()),
        Ok(Err(reason)) => (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("not ready: {reason}\n"),
        ),
        Err(e) => (StatusCode::SERVICE_UNAVAILABLE, format!("not ready: {e}\n")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn following() -> ReadySnapshot {
        ReadySnapshot {
            phase: Phase::Following,
            unbound: Vec::new(),
            in_ibd: false,
            blocks: 100,
            headers: 100,
            tip_age_secs: Some(0),
            max_tip_age_secs: 24 * 60 * 60,
            sh_lag: None,
        }
    }

    #[test]
    fn readiness_reports_the_first_failing_gate() {
        let mut cases = Vec::new();
        for phase in Phase::ALL {
            let want = match phase {
                Phase::Following => Ok(()),
                other => Err(other.as_str().to_string()),
            };
            cases.push((
                ReadySnapshot {
                    phase,
                    in_ibd: phase != Phase::Following,
                    ..following()
                },
                want,
            ));
        }
        let unbound = |names: &[&'static str]| ReadySnapshot {
            unbound: names.to_vec(),
            in_ibd: true,
            ..following()
        };
        cases.push((unbound(&["rpc"]), Err("rpc not listening".into())));
        cases.push((
            unbound(&["electrum", "esplora"]),
            Err("electrum, esplora not listening".into()),
        ));
        cases.push((
            ReadySnapshot {
                in_ibd: true,
                headers: 200,
                ..following()
            },
            Err("initial block download".into()),
        ));
        let behind = |n: u32, sh_lag: Option<u32>| ReadySnapshot {
            headers: 100 + n,
            sh_lag,
            ..following()
        };
        cases.push((behind(READY_LAG_BLOCKS, None), Ok(())));
        cases.push((
            behind(READY_LAG_BLOCKS + 1, Some(0)),
            Err(format!(
                "tip {} blocks behind headers",
                READY_LAG_BLOCKS + 1
            )),
        ));
        cases.push((behind(0, Some(READY_LAG_BLOCKS)), Ok(())));
        cases.push((
            behind(0, Some(READY_LAG_BLOCKS + 1)),
            Err(format!(
                "scripthash index {} blocks behind tip",
                READY_LAG_BLOCKS + 1
            )),
        ));
        cases.push((
            ReadySnapshot {
                headers: 90,
                ..following()
            },
            Ok(()),
        ));
        let stale = |age: u64| ReadySnapshot {
            tip_age_secs: Some(age),
            ..following()
        };
        cases.push((stale(24 * 60 * 60), Ok(())));
        cases.push((
            stale(24 * 60 * 60 + 1),
            Err("tip stale (last block 86401s ago)".into()),
        ));
        cases.push((
            ReadySnapshot {
                tip_age_secs: None,
                ..following()
            },
            Ok(()),
        ));
        for (snap, want) in cases {
            assert_eq!(readiness(&snap), want, "{snap:?}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn opening_node_is_live_but_not_ready() {
        let addr = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap()
        };
        let _h = run_health(addr, NodeStatus::new(Network::Regtest, false), false)
            .await
            .unwrap();
        assert_eq!(get(addr, "/healthz").await, "HTTP/1.1 200 OK|ok\n");
        assert_eq!(
            get(addr, "/readyz").await,
            "HTTP/1.1 503 Service Unavailable|not ready: opening\n"
        );
    }

    async fn get(addr: SocketAddr, path: &str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        let req = format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
        s.write_all(req.as_bytes()).await.unwrap();
        let mut buf = String::new();
        s.read_to_string(&mut buf).await.unwrap();
        let (head, body) = buf.split_once("\r\n\r\n").unwrap();
        format!("{}|{body}", head.lines().next().unwrap())
    }
}
