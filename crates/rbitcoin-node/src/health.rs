//! Health listener (`--health-listen`): unauthenticated `GET /healthz` for
//! process probes.
//!
//! It binds at the top of [`crate::run_p2p`], before the store opens, so it
//! answers through a schema migration, catch-up, and index materialize. The
//! RPC, Electrum, and Esplora listeners bind only after those.

use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;
use rbitcoin_log::{info, warn};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;

/// Probes in flight at once. Probers send one request per period.
const MAX_CONCURRENT: usize = 16;
/// Per-request wall. Probe timeouts are usually 1s.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Bound health listener. Dropping it stops serving.
pub(crate) struct HealthHandle {
    task: JoinHandle<()>,
}

impl Drop for HealthHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Bind `addr` and serve the health routes until the handle drops.
pub(crate) async fn run_health(addr: SocketAddr) -> std::io::Result<HealthHandle> {
    let listener = TcpListener::bind(addr).await?;
    let local_addr = listener.local_addr()?;
    if !local_addr.ip().is_loopback() {
        warn!("health: {local_addr} is not loopback; /healthz is unauthenticated");
    }
    let app = router();
    let task = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            warn!("health: serve ended: {e}");
        }
    });
    info!("health HTTP on {local_addr}");
    Ok(HealthHandle { task })
}

fn router() -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        // Outer → inner: concurrency → body (GET only, so none) → timeout.
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(RequestBodyLimitLayer::new(0))
        .layer(ConcurrencyLimitLayer::new(MAX_CONCURRENT))
}

async fn healthz() -> &'static str {
    "ok\n"
}
