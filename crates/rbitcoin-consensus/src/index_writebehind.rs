//! Post-IBD block index write-behind (`rbtc-idx-wb`): BIP158 basic filters
//! and BIP-352 tweaks from one read of Class A.
//!
//! The IO thread plans windows of consecutive heights from the lower index
//! watermark up to the released tip and reads each through
//! [`rbitcoin_store::read_index_window`] (one completion session). One CPU
//! worker builds each index for the heights that index still needs (tweak
//! EC math on idle script workers) and commits each index once per window
//! under the index write-behind lock. A commit that finds its watermark or a
//! `confirmed[h]` moved (a reorg) returns 0, and the IO thread re-plans from
//! the watermarks. A wide gap is the materialize; at the tip each release is
//! a one-height window.

use crate::silent_payments::tweak_records_from_window;
use crate::ConsensusError;
use rbitcoin_primitives::{Fk, Height};
use rbitcoin_query::Query;
use rbitcoin_store::IndexWindow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Heights per window. Bounds how long a disconnect waits and how far the
/// IO thread reads ahead of commits.
const WINDOW_HEIGHTS: u32 = 64;
/// Creates per window. RAM trade: a window holds its blocks' outputs, input
/// edges, P2TR-output witnesses, and spent parents (tens of MB at mainnet
/// sizes); at most two windows wait for the CPU worker.
const WINDOW_TXS: u64 = 50_000;
const PROGRESS_EVERY: Duration = Duration::from_secs(10);

/// Lowest height either index still needs, if any index is on.
fn next_needed(query: &Query) -> Option<u32> {
    match (query.filter_index_next(), query.tweak_index_next()) {
        (Some(f), Some(t)) => Some(f.min(t)),
        (f, t) => f.or(t),
    }
}

/// Heights `start..` for the next window, capped at `target`.
fn plan_window(query: &Query, start: u32, target: u32) -> Result<u32, ConsensusError> {
    let mut end = start;
    let mut txs = 0u64;
    while end < target && end - start + 1 < WINDOW_HEIGHTS {
        let fk = query.store().confirmed.get(Height(end + 1))?.ok_or(
            rbitcoin_store::StoreError::Corrupt("invariant: index height not confirmed"),
        )?;
        let n = query
            .store()
            .header_txs
            .get_range(fk)?
            .map_or(0, |(_, n)| u64::from(n));
        if txs + n > WINDOW_TXS {
            break;
        }
        txs += n;
        end += 1;
    }
    Ok(end)
}

fn read_window(query: &Query, start: u32, end: u32) -> Result<IndexWindow, ConsensusError> {
    let tweaks_from = query.tweak_index_next();
    let heights = query.index_heights(start, end, tweaks_from)?;
    Ok(query.read_index_window(&heights)?)
}

/// Build and commit what each index still needs from `window`. Returns
/// false when a commit found the watermark or a `confirmed[h]` moved.
fn commit_window(query: &Query, window: &IndexWindow) -> Result<bool, ConsensusError> {
    let t0 = Instant::now();
    let Some(first) = window.blocks.first().map(|b| b.height.0) else {
        return Ok(true);
    };
    let mut ok = true;
    if let Some(next) = query.filter_index_next() {
        let skip = next.saturating_sub(first) as usize;
        let built = window
            .blocks
            .iter()
            .enumerate()
            .skip(skip)
            .map(|(i, b)| Ok((query.basic_filter_from_window(window, i)?, b.header_fk)))
            .collect::<Result<Vec<_>, ConsensusError>>()?;
        if !built.is_empty() {
            ok &= query.commit_window_filters(first + skip as u32, &built)? > 0;
        }
    }
    if let Some(next) = query.tweak_index_next() {
        let skip = next.saturating_sub(first) as usize;
        let items = window
            .blocks
            .iter()
            .enumerate()
            .skip(skip)
            .map(|(i, b)| Ok((b.height, b.header_fk, tweak_records_from_window(window, i)?)))
            .collect::<Result<Vec<(Height, Fk, _)>, ConsensusError>>()?;
        if !items.is_empty() {
            ok &= query.commit_window_tweaks(&items)? > 0;
        }
    }
    rbitcoin_query::note_confirm(
        &query.confirm_stats().blockfilter_ns,
        t0.elapsed().as_nanos() as u64,
    );
    Ok(ok)
}

/// Seal every released height on this thread (regtest `generate`, tests).
pub fn build_indexes_released(query: &Query) -> Result<(), ConsensusError> {
    while let (Some(start), Some(target)) = (next_needed(query), query.index_target()) {
        if start > target {
            break;
        }
        let end = plan_window(query, start, target)?;
        if !commit_window(query, &read_window(query, start, end)?)? {
            break;
        }
    }
    Ok(())
}

fn cpu_worker(
    query: &Query,
    rx: Receiver<IndexWindow>,
    resync: &AtomicBool,
) -> Result<(), ConsensusError> {
    for window in rx {
        if resync.load(Ordering::Acquire) {
            continue;
        }
        if !commit_window(query, &window)? {
            resync.store(true, Ordering::Release);
        }
    }
    Ok(())
}

/// Spawn `rbtc-idx-wb` once tip mode is entered. Stop is checked between
/// windows; apply errors request `stop` and `on_fatal`. `on_filters_caught_up`
/// runs once, the first time filters reach the released tip.
pub fn spawn_index_writebehind(
    query: Arc<Query>,
    stop: Arc<AtomicBool>,
    on_fatal: impl FnOnce() + Send + 'static,
    on_filters_caught_up: impl FnOnce() + Send + 'static,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("rbtc-idx-wb".into())
        .spawn(move || {
            if let Err(e) = run_writebehind(&query, &stop, on_filters_caught_up) {
                rbitcoin_log::error!("index write-behind: {e}");
                stop.store(true, Ordering::SeqCst);
                on_fatal();
            }
        })
        .expect("spawn index write-behind")
}

fn run_writebehind(
    query: &Query,
    stop: &AtomicBool,
    on_filters_caught_up: impl FnOnce(),
) -> Result<(), ConsensusError> {
    // Spawned after catch-up: the current tip was already announced.
    if let Some(tip) = query.tip_height() {
        query.release_index_writebehind(tip);
    }
    let resync = AtomicBool::new(false);
    let (tx, rx) = sync_channel::<IndexWindow>(2);
    std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name("rbtc-idx-cpu".into())
            .spawn_scoped(scope, || cpu_worker(query, rx, &resync))
            .expect("spawn index cpu worker");
        let io = io_loop(query, stop, &resync, &tx, on_filters_caught_up);
        drop(tx);
        let cpu = worker.join().expect("index cpu worker");
        io.and(cpu)
    })
}

struct Pass {
    from: u32,
    started: Instant,
    last_log: Instant,
}

fn io_loop(
    query: &Query,
    stop: &AtomicBool,
    resync: &AtomicBool,
    tx: &std::sync::mpsc::SyncSender<IndexWindow>,
    on_filters_caught_up: impl FnOnce(),
) -> Result<(), ConsensusError> {
    let mut on_filters_caught_up = Some(on_filters_caught_up);
    let mut cursor: Option<u32> = None;
    let mut pass: Option<Pass> = None;
    while !stop.load(Ordering::Relaxed) {
        if resync.swap(false, Ordering::AcqRel) {
            cursor = None;
        }
        let target = query.index_target();
        if let (Some(t), Some(f)) = (target, query.filter_index_next()) {
            if f > t {
                if let Some(cb) = on_filters_caught_up.take() {
                    rbitcoin_log::info!(
                        "blockfilter: caught up through={}; advertising NODE_COMPACT_FILTERS",
                        f - 1
                    );
                    cb();
                }
            }
        }
        let start = match cursor {
            Some(c) => Some(c),
            None => next_needed(query),
        };
        let (Some(start), Some(target)) = (start, target) else {
            query.wait_index_release(Duration::from_millis(200));
            continue;
        };
        if start > target {
            if let Some(p) = pass.take() {
                rbitcoin_log::info!(
                    "index: build done through={target} heights={} elapsed={:.1}s",
                    target + 1 - p.from,
                    p.started.elapsed().as_secs_f64()
                );
            }
            query.wait_index_release(Duration::from_millis(200));
            continue;
        }
        let now = Instant::now();
        if target - start >= WINDOW_HEIGHTS && pass.is_none() {
            rbitcoin_log::info!(
                "index: build from={start} to={target} filters={:?} tweaks={:?}",
                query.filter_index_next(),
                query.tweak_index_next()
            );
            pass = Some(Pass {
                from: start,
                started: now,
                last_log: now,
            });
        }
        if let Some(p) = pass.as_mut() {
            if p.last_log.elapsed() >= PROGRESS_EVERY {
                let secs = p.started.elapsed().as_secs_f64();
                rbitcoin_log::info!(
                    "index: build next={start} tip={target} rate={:.0}/s remain={} elapsed={secs:.0}s",
                    f64::from(start - p.from) / secs.max(0.001),
                    target + 1 - start
                );
                p.last_log = now;
            }
        }
        let end = plan_window(query, start, target)?;
        let window = read_window(query, start, end)?;
        if pass.is_none() {
            rbitcoin_log::info!(
                "index: apply h={start}..={end} read={}ms",
                now.elapsed().as_millis()
            );
        }
        if tx.send(window).is_err() {
            break;
        }
        cursor = Some(end + 1);
    }
    Ok(())
}
