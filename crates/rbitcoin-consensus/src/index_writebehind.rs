//! Post-IBD block index write-behind (`rbtc-idx-wb`): BIP158 basic filters
//! and BIP-352 tweaks from one read of Class A.
//!
//! The IO thread plans windows of consecutive heights from the lower index
//! watermark up to the released tip and reads each through
//! [`rbitcoin_store::read_index_window`] (one completion session). One CPU
//! worker builds each index for the heights that index still needs (tweak
//! EC math included) and commits each index once per window
//! under the index write-behind lock. A commit that finds its watermark or a
//! `confirmed[h]` moved (a reorg) returns 0, and the IO thread re-plans from
//! the watermarks. A wide gap is the materialize; at the tip each release is
//! a one-height window.

use crate::silent_payments::tweak_records_from_window;
use crate::ConsensusError;
use rbitcoin_primitives::{Fk, Height};
use rbitcoin_query::Query;
use rbitcoin_store::IndexWindow;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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

/// Milliseconds of one interval, or of one tip window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct IndexStageSample {
    read_ms: u64,
    build_ms: u64,
    commit_ms: u64,
}

/// Interval sums for `index: build` / `index: apply`. Not an `ibd: perf` token.
#[derive(Debug, Default)]
struct IndexStageMs {
    read_ns: AtomicU64,
    build_ns: AtomicU64,
    commit_ns: AtomicU64,
}

impl IndexStageMs {
    fn add_ns(slot: &AtomicU64, ns: u64) {
        if ns > 0 {
            slot.fetch_add(ns, Ordering::Relaxed);
        }
    }

    fn add_read(&self, ns: u64) {
        Self::add_ns(&self.read_ns, ns);
    }

    fn add_build(&self, ns: u64) {
        Self::add_ns(&self.build_ns, ns);
    }

    fn add_commit(&self, ns: u64) {
        Self::add_ns(&self.commit_ns, ns);
    }

    /// Take the interval and reset. Sub-millisecond leftovers truncate.
    fn take_ms(&self) -> IndexStageSample {
        IndexStageSample {
            read_ms: self.read_ns.swap(0, Ordering::Relaxed) / 1_000_000,
            build_ms: self.build_ns.swap(0, Ordering::Relaxed) / 1_000_000,
            commit_ms: self.commit_ns.swap(0, Ordering::Relaxed) / 1_000_000,
        }
    }
}

struct IndexBuildProgress {
    next: u32,
    tip: u32,
    from: u32,
    elapsed: Duration,
    stages: IndexStageSample,
}

fn format_index_build_progress(p: &IndexBuildProgress) -> String {
    let secs = p.elapsed.as_secs_f64();
    let rate = f64::from(p.next.saturating_sub(p.from)) / secs.max(0.001);
    format!(
        "index: build next={} tip={} rate={:.0}/s remain={} elapsed={:.0}s read={}ms build={}ms commit={}ms",
        p.next,
        p.tip,
        rate,
        p.tip.saturating_add(1).saturating_sub(p.next),
        secs,
        p.stages.read_ms,
        p.stages.build_ms,
        p.stages.commit_ms,
    )
}

fn format_index_apply(start: u32, end: u32, stages: IndexStageSample) -> String {
    format!(
        "index: apply h={start}..={end} read={}ms build={}ms commit={}ms",
        stages.read_ms, stages.build_ms, stages.commit_ms
    )
}

struct CommitTimes {
    ok: bool,
    build_ns: u64,
    commit_ns: u64,
}

/// Build and commit what each index still needs from `window`. `ok` is false
/// when a commit found the watermark or a `confirmed[h]` moved.
fn commit_window(
    query: &Query,
    window: &IndexWindow,
    stages: &IndexStageMs,
) -> Result<CommitTimes, ConsensusError> {
    let Some(first) = window.blocks.first().map(|b| b.height.0) else {
        return Ok(CommitTimes {
            ok: true,
            build_ns: 0,
            commit_ns: 0,
        });
    };
    let t_build = Instant::now();
    let filters = if let Some(next) = query.filter_index_next() {
        let skip = next.saturating_sub(first) as usize;
        let built = window
            .blocks
            .iter()
            .enumerate()
            .skip(skip)
            .map(|(i, b)| Ok((query.basic_filter_from_window(window, i)?, b.header_fk)))
            .collect::<Result<Vec<_>, ConsensusError>>()?;
        Some((first + skip as u32, built))
    } else {
        None
    };
    let tweaks = if let Some(next) = query.tweak_index_next() {
        let skip = next.saturating_sub(first) as usize;
        let items = window
            .blocks
            .iter()
            .enumerate()
            .skip(skip)
            .map(|(i, b)| Ok((b.height, b.header_fk, tweak_records_from_window(window, i)?)))
            .collect::<Result<Vec<(Height, Fk, _)>, ConsensusError>>()?;
        Some(items)
    } else {
        None
    };
    let build_ns = t_build.elapsed().as_nanos() as u64;
    let t_commit = Instant::now();
    let committed = (|| {
        let mut ok = true;
        if let Some((start, built)) = &filters {
            if !built.is_empty() {
                ok &= query.commit_window_filters(*start, built)? > 0;
            }
        }
        if let Some(items) = &tweaks {
            if !items.is_empty() {
                ok &= query.commit_window_tweaks(items)? > 0;
            }
        }
        Ok(ok)
    })();
    let commit_ns = t_commit.elapsed().as_nanos() as u64;
    stages.add_build(build_ns);
    stages.add_commit(commit_ns);
    rbitcoin_query::note_confirm(
        &query.confirm_stats().blockfilter_ns,
        build_ns.saturating_add(commit_ns),
    );
    committed.map(|ok| CommitTimes {
        ok,
        build_ns,
        commit_ns,
    })
}

/// Seal every released height on this thread (regtest `generate`, tests).
pub fn build_indexes_released(query: &Query) -> Result<(), ConsensusError> {
    while let (Some(start), Some(target)) = (next_needed(query), query.index_target()) {
        if start > target {
            break;
        }
        let end = plan_window(query, start, target)?;
        let stages = IndexStageMs::default();
        if !commit_window(query, &read_window(query, start, end)?, &stages)?.ok {
            break;
        }
    }
    Ok(())
}

struct ReadyWindow {
    window: IndexWindow,
    start: u32,
    end: u32,
    read_ns: u64,
    /// Tip path: one height, logged after commit with this window's times.
    log_apply: bool,
}

fn cpu_worker(
    query: &Query,
    rx: Receiver<ReadyWindow>,
    resync: &AtomicBool,
    stages: &IndexStageMs,
) -> Result<(), ConsensusError> {
    for ready in rx {
        if resync.load(Ordering::Acquire) {
            continue;
        }
        let times = commit_window(query, &ready.window, stages)?;
        if ready.log_apply {
            rbitcoin_log::info!(
                "{}",
                format_index_apply(
                    ready.start,
                    ready.end,
                    IndexStageSample {
                        read_ms: ready.read_ns / 1_000_000,
                        build_ms: times.build_ns / 1_000_000,
                        commit_ms: times.commit_ns / 1_000_000,
                    },
                )
            );
        }
        if !times.ok {
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
    let stages = IndexStageMs::default();
    let (tx, rx) = sync_channel::<ReadyWindow>(2);
    std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name("rbtc-idx-cpu".into())
            .spawn_scoped(scope, || cpu_worker(query, rx, &resync, &stages))
            .expect("spawn index cpu worker");
        let io = io_loop(query, stop, &resync, &stages, &tx, on_filters_caught_up);
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
    stages: &IndexStageMs,
    tx: &std::sync::mpsc::SyncSender<ReadyWindow>,
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
                rbitcoin_log::info!(
                    "{}",
                    format_index_build_progress(&IndexBuildProgress {
                        next: start,
                        tip: target,
                        from: p.from,
                        elapsed: p.started.elapsed(),
                        stages: stages.take_ms(),
                    })
                );
                p.last_log = now;
            }
        }
        let end = plan_window(query, start, target)?;
        let t_read = Instant::now();
        let window = read_window(query, start, end)?;
        let read_ns = t_read.elapsed().as_nanos() as u64;
        stages.add_read(read_ns);
        if tx
            .send(ReadyWindow {
                window,
                start,
                end,
                read_ns,
                log_apply: pass.is_none(),
            })
            .is_err()
        {
            break;
        }
        cursor = Some(end + 1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_build_progress_names_stage_ms() {
        let line = format_index_build_progress(&IndexBuildProgress {
            next: 800_000,
            tip: 900_000,
            from: 700_000,
            elapsed: Duration::from_secs(100),
            stages: IndexStageSample {
                read_ms: 4_000,
                build_ms: 5_500,
                commit_ms: 12,
            },
        });
        assert!(line.contains("next=800000"), "{line}");
        assert!(line.contains("tip=900000"), "{line}");
        assert!(line.contains("rate=1000/s"), "{line}");
        assert!(line.contains("remain=100001"), "{line}");
        assert!(line.contains("elapsed=100s"), "{line}");
        assert!(line.contains("read=4000ms"), "{line}");
        assert!(line.contains("build=5500ms"), "{line}");
        assert!(line.contains("commit=12ms"), "{line}");
    }

    #[test]
    fn index_apply_names_stage_ms() {
        let line = format_index_apply(
            5,
            5,
            IndexStageSample {
                read_ms: 20,
                build_ms: 30,
                commit_ms: 4,
            },
        );
        assert_eq!(line, "index: apply h=5..=5 read=20ms build=30ms commit=4ms");
    }

    #[test]
    fn stage_sample_take_resets() {
        let s = IndexStageMs::default();
        s.add_read(2_500_000);
        s.add_build(1_500_000);
        s.add_commit(500_000);
        let a = s.take_ms();
        assert_eq!((a.read_ms, a.build_ms, a.commit_ms), (2, 1, 0));
        let b = s.take_ms();
        assert_eq!((b.read_ms, b.build_ms, b.commit_ms), (0, 0, 0));
    }
}
