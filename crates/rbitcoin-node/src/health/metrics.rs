//! Prometheus text exposition for `--metrics`. Each gauge is a value RPC or a
//! log line already publishes, named after that source; `phase` and `ready`
//! are `/readyz` itself.

use super::{readiness, NodeStatus, Phase};
use rbitcoin_net::ChainHub;
use std::fmt::{Display, Write};
use std::time::UNIX_EPOCH;

pub(super) const CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// One scrape. Chain reads may touch the store and the mempool totals take
/// its lock, so call from the blocking pool. Cost: one peer snapshot and one
/// fold over the mempool (the same fold as `getmempoolinfo`).
pub(super) fn render(status: &NodeStatus) -> String {
    let mut out = Exposition::default();
    out.family(
        "rbitcoin_build_info",
        "gauge",
        "Version and network of this node.",
    );
    out.sample(
        "rbitcoin_build_info",
        &format!(
            "{{version=\"{}\",network=\"{}\"}}",
            env!("CARGO_PKG_VERSION"),
            status.network.as_str()
        ),
        1,
    );
    let phase = status.phase();
    out.family(
        "rbitcoin_phase",
        "gauge",
        "Bring-up phase, as /readyz names it.",
    );
    for p in Phase::ALL {
        out.sample(
            "rbitcoin_phase",
            &format!("{{phase=\"{}\"}}", p.as_str()),
            u8::from(p == phase),
        );
    }
    out.gauge(
        "rbitcoin_ready",
        "1 when /readyz answers 200.",
        u8::from(readiness(&status.ready_snapshot()).is_ok()),
    );
    out.gauge(
        "process_start_time_seconds",
        "Start time of the process since the Unix epoch in seconds.",
        status
            .started
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    );
    let rss_kb = rbitcoin_net::read_platform_rss().rss_kb;
    if rss_kb > 0 {
        out.gauge(
            "process_resident_memory_bytes",
            "Resident memory size in bytes (ibd: sizes rss=).",
            rss_kb * 1024,
        );
    }
    if let Some(chain) = status.chain.get() {
        chain_gauges(&mut out, chain, status.sh_index);
    }
    if let Some(peers) = status.peers.get() {
        let (inbound, outbound) = rbitcoin_net::connection_counts(&peers.snapshot());
        out.family(
            "rbitcoin_connections",
            "gauge",
            "Peer connections (getnetworkinfo.connections_in / connections_out).",
        );
        out.sample("rbitcoin_connections", "{direction=\"in\"}", inbound);
        out.sample("rbitcoin_connections", "{direction=\"out\"}", outbound);
    }
    if let Some(mempool) = status.mempool.get() {
        let (size, vbytes, _fee) = mempool.live_adjusted_totals();
        out.gauge(
            "rbitcoin_mempool_transactions",
            "Mempool transactions (getmempoolinfo.size).",
            size,
        );
        out.gauge(
            "rbitcoin_mempool_bytes",
            "Sum of mempool virtual sizes (getmempoolinfo.bytes).",
            vbytes,
        );
    }
    out.0
}

fn chain_gauges(out: &mut Exposition, chain: &ChainHub, sh_index: bool) {
    let tip = chain.query.tip_height();
    out.gauge(
        "rbitcoin_blocks",
        "Active chain height (getblockchaininfo.blocks).",
        tip.map_or(0, |h| h.0),
    );
    out.gauge(
        "rbitcoin_headers",
        "Best header height (getblockchaininfo.headers).",
        chain.best_header_height(),
    );
    let time = tip
        .and_then(|h| chain.query.header_at_height(h).ok().flatten())
        .map_or(0, |(_, rec)| rec.timestamp);
    out.gauge(
        "rbitcoin_tip_time_seconds",
        "Tip block time (getblockchaininfo.time).",
        time,
    );
    out.gauge(
        "rbitcoin_initial_block_download",
        "1 during initial block download (getblockchaininfo.initialblockdownload).",
        u8::from(chain.in_ibd()),
    );
    if sh_index {
        out.gauge(
            "rbitcoin_scripthash_lag_blocks",
            "Blocks the scripthash index trails the tip (tip: accept sh_lag=).",
            chain.query.sh_lag_heights(),
        );
    }
}

#[derive(Default)]
struct Exposition(String);

impl Exposition {
    fn family(&mut self, name: &str, kind: &str, help: &str) {
        let _ = writeln!(self.0, "# HELP {name} {help}\n# TYPE {name} {kind}");
    }

    fn sample(&mut self, name: &str, labels: &str, value: impl Display) {
        let _ = writeln!(self.0, "{name}{labels} {value}");
    }

    fn gauge(&mut self, name: &str, help: &str, value: impl Display) {
        self.family(name, "gauge", help);
        self.sample(name, "", value);
    }
}
