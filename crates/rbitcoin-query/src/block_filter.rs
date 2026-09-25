//! BIP158 basic filters over [`rbitcoin_store::BlockFilterTable`].
//!
//! Additive index under the store dir; Class A is unchanged. Filters are
//! built from Class A (output scripts plus spent prevout scripts through
//! `input.body` parent edges), so heights below a seqsigwit prune still build.
//! The watermark is the last committed height.

use bitcoin::bip158::{BlockFilter, FilterHash, FilterHeader, GcsFilterWriter};
use bitcoin::hashes::Hash;
use rbitcoin_primitives::{Fk, Height};
use rbitcoin_store::{BlockFilterRecord, BlockFilterSlot, BlockFilterTable, StoreError};

use crate::{Query, QueryError};

/// BIP158 basic filter Golomb-Rice parameters (`M`, `P`).
const BASIC_FILTER_M: u64 = 784_931;
const BASIC_FILTER_P: u8 = 19;
/// Heights built per commit. Bounds RAM held between build and commit and
/// how long a disconnect waits on the appender lock.
const SEAL_CHUNK: u32 = 64;

/// Block filter write-behind state (one appender thread).
pub(crate) struct BlockFilterWriteBehind {
    /// Serializes commits against disconnect truncate.
    appender: std::sync::Mutex<()>,
    wake: std::sync::Mutex<()>,
    wake_cv: std::sync::Condvar,
}

impl BlockFilterWriteBehind {
    pub(crate) fn new() -> Self {
        Self {
            appender: std::sync::Mutex::new(()),
            wake: std::sync::Mutex::new(()),
            wake_cv: std::sync::Condvar::new(),
        }
    }

    pub(crate) fn notify(&self) {
        self.wake_cv.notify_one();
    }

    pub(crate) fn lock_appender(&self) -> std::sync::MutexGuard<'_, ()> {
        self.appender.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn wait(&self, d: std::time::Duration) {
        let g = self.wake.lock().unwrap_or_else(|e| e.into_inner());
        let _ = self.wake_cv.wait_timeout(g, d);
    }
}

impl Query {
    /// Basic filter of best-chain `height`, built from Class A, and its `header_fk`.
    pub fn build_basic_filter(&self, height: Height) -> Result<(BlockFilter, Fk), QueryError> {
        let (header_fk, rec) = self.header_at_height(height)?.ok_or(StoreError::Corrupt(
            "invariant: blockfilter height not confirmed",
        ))?;
        let (first, n) = self
            .store
            .header_txs
            .get_range(header_fk)?
            .ok_or(StoreError::Corrupt("confirmed header missing body list"))?;
        let elements = self
            .store
            .txs
            .basic_filter_elements(first.0, first.0 + u64::from(n) - 1)?;
        let k0 = u64::from_le_bytes(rec.hash[0..8].try_into().unwrap());
        let k1 = u64::from_le_bytes(rec.hash[8..16].try_into().unwrap());
        let mut content = Vec::new();
        let mut writer = GcsFilterWriter::new(&mut content, k0, k1, BASIC_FILTER_M, BASIC_FILTER_P);
        for e in &elements {
            writer.add_element(e);
        }
        writer
            .finish()
            .map_err(|_| StoreError::Corrupt("invariant: blockfilter encode"))?;
        Ok((BlockFilter::new(&content), header_fk))
    }

    pub fn block_filter_enabled(&self) -> bool {
        self.block_filter_enabled
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Turn the index on (opening or creating the table) or off.
    ///
    /// Open drops committed slots that are not on the best chain: above the
    /// tip, or whose `header_fk` is not `confirmed[h]` (a crash between a
    /// disconnect and its filter truncate, or a torn slot).
    pub fn set_block_filter_index(&self, enabled: bool) -> Result<(), QueryError> {
        if enabled && self.block_filters.get().is_none() {
            let table = BlockFilterTable::open_or_create(self.store.path())?;
            self.trim_block_filters_to_best_chain(&table)?;
            let _ = self.block_filters.set(table);
        }
        self.block_filter_enabled
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    fn trim_block_filters_to_best_chain(&self, table: &BlockFilterTable) -> Result<(), QueryError> {
        table.truncate_through(self.tip_height())?;
        let was = table.next_height().0;
        let mut keep = was.checked_sub(1);
        while let Some(h) = keep {
            let slot = table.slot(Height(h))?.map(|s| s.header_fk);
            if slot.is_some() && slot == self.store.confirmed.get(Height(h))? {
                break;
            }
            keep = h.checked_sub(1);
        }
        if keep.map_or(0, |h| h + 1) != was {
            rbitcoin_log::warn!(
                "blockfilter: dropping slots off the best chain: keep through {keep:?} (had {was})"
            );
            table.truncate_through(keep.map(Height))?;
        }
        Ok(())
    }

    fn block_filter_table(&self) -> Option<&BlockFilterTable> {
        if self.block_filter_enabled() {
            self.block_filters.get()
        } else {
            None
        }
    }

    /// Last committed filter height.
    pub fn basic_filter_hwm(&self) -> Result<Option<u32>, QueryError> {
        Ok(self
            .block_filter_table()
            .and_then(|t| t.next_height().0.checked_sub(1)))
    }

    /// Filter bytes and header at `height`. `None` past the watermark.
    pub fn basic_filter_at(
        &self,
        height: u32,
    ) -> Result<Option<(Vec<u8>, FilterHeader)>, QueryError> {
        let Some(table) = self.block_filter_table() else {
            return Ok(None);
        };
        Ok(table
            .filter(Height(height))?
            .map(|(body, slot)| (body, FilterHeader::from_byte_array(slot.filter_header))))
    }

    /// Heights the appender may seal now: released through tip, or `None`.
    fn block_filter_target(&self) -> Option<u32> {
        let tip = self.tip_height()?.0;
        Some(self.index_released_through_height()?.min(tip))
    }

    fn build_basic_filters(
        &self,
        start: u32,
        end: u32,
    ) -> Result<Vec<(BlockFilter, Fk)>, QueryError> {
        (start..=end)
            .map(|h| self.build_basic_filter(Height(h)))
            .collect()
    }

    /// Commit filters built for `start..` under the appender lock. Returns
    /// heights committed: 0 when the watermark moved or a built height is no
    /// longer `confirmed[h]` (a disconnect in between drops the chunk).
    fn commit_basic_filters(
        &self,
        table: &BlockFilterTable,
        start: u32,
        built: &[(BlockFilter, Fk)],
    ) -> Result<u32, QueryError> {
        let _appender = self.bf_wb.lock_appender();
        if table.next_height().0 != start {
            return Ok(0);
        }
        for (h, (_, header_fk)) in (start..).zip(built) {
            if self.store.confirmed.get(Height(h))? != Some(*header_fk) {
                return Ok(0);
            }
        }
        let mut prev = match start.checked_sub(1) {
            None => FilterHeader::from_byte_array([0u8; 32]),
            Some(h) => FilterHeader::from_byte_array(
                table
                    .slot(Height(h))?
                    .ok_or(StoreError::Corrupt(
                        "invariant: blockfilter slot below next",
                    ))?
                    .filter_header,
            ),
        };
        let mut recs = Vec::with_capacity(built.len());
        for (h, (filter, header_fk)) in (start..).zip(built) {
            let header = filter.filter_header(&prev);
            recs.push(BlockFilterRecord {
                height: Height(h),
                slot: BlockFilterSlot {
                    header_fk: *header_fk,
                    filter_hash: FilterHash::hash(&filter.content).to_byte_array(),
                    filter_header: header.to_byte_array(),
                },
                filter: &filter.content,
            });
            prev = header;
        }
        table.put(&recs)?;
        Ok(built.len() as u32)
    }

    /// Seal the next chunk of `[next, through]` on this thread. Returns
    /// heights committed.
    pub fn seal_block_filters_chunk(&self, through: u32) -> Result<u32, QueryError> {
        let Some(table) = self.block_filter_table() else {
            return Ok(0);
        };
        let start = table.next_height().0;
        if start > through {
            return Ok(0);
        }
        let end = through.min(start.saturating_add(SEAL_CHUNK - 1));
        let built = self.build_basic_filters(start, end)?;
        self.commit_basic_filters(table, start, &built)
    }

    /// Materialize step: build up to `workers` chunks of `[next, through]` in
    /// parallel, then commit them in height order, stopping between commits.
    ///
    /// Trade: RAM holds `workers × SEAL_CHUNK` built filters (tens of MB at
    /// mainnet sizes) and `workers` threads read Class A at once, only while
    /// a gap is being closed.
    fn seal_block_filters_parallel(
        &self,
        through: u32,
        workers: u32,
        stop: &std::sync::atomic::AtomicBool,
    ) -> Result<u32, QueryError> {
        let Some(table) = self.block_filter_table() else {
            return Ok(0);
        };
        let start = table.next_height().0;
        if start > through {
            return Ok(0);
        }
        let chunks: Vec<(u32, u32)> = (0..workers)
            .map(|i| start.saturating_add(i * SEAL_CHUNK))
            .take_while(|&s| s <= through)
            .map(|s| (s, through.min(s.saturating_add(SEAL_CHUNK - 1))))
            .collect();
        let built = std::thread::scope(|scope| {
            let handles: Vec<_> = chunks
                .iter()
                .map(|&(s, e)| scope.spawn(move || self.build_basic_filters(s, e)))
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("block filter build worker"))
                .collect::<Result<Vec<_>, _>>()
        })?;
        let mut done = 0;
        for (&(s, _), chunk) in chunks.iter().zip(&built) {
            if stop.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            let n = self.commit_basic_filters(table, s, chunk)?;
            done += n;
            if n == 0 {
                break;
            }
        }
        Ok(done)
    }

    /// Seal every released height now (regtest `generate`, tests).
    pub fn seal_block_filters_released(&self) -> Result<(), QueryError> {
        while let Some(t) = self.block_filter_target() {
            if self.seal_block_filters_chunk(t)? == 0 {
                break;
            }
        }
        Ok(())
    }

    pub fn truncate_basic_filters_to_tip(&self) -> Result<(), QueryError> {
        match self.block_filters.get() {
            Some(t) => Ok(t.truncate_through(self.tip_height())?),
            None => Ok(()),
        }
    }
}

/// Progress of one materialize pass (a gap wider than one chunk).
struct Materialize {
    from: u32,
    started: std::time::Instant,
    last_log: std::time::Instant,
}

/// The block filter appender: after catch-up, seals `(hwm, released tip]`
/// off the confirm path. A gap wider than one chunk is the materialize
/// (parallel build, progress logs); at the tip each release is one chunk.
///
/// Stop is checked between commits, so shutdown waits at most one chunk
/// build. Apply errors request `stop` and `on_fatal`, like the scripthash
/// appender.
pub fn spawn_block_filter_writebehind(
    query: std::sync::Arc<Query>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    on_fatal: impl FnOnce() + Send + 'static,
) -> std::thread::JoinHandle<()> {
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};
    const PROGRESS_EVERY: Duration = Duration::from_secs(10);
    let workers = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1)
        .clamp(1, 8);
    std::thread::Builder::new()
        .name("rbtc-bf-wb".into())
        .spawn(move || {
            // Spawned after catch-up: the current tip was already announced.
            if let Some(tip) = query.tip_height() {
                query.release_index_writebehind(tip);
            }
            let mut pass: Option<Materialize> = None;
            while !stop.load(Ordering::Relaxed) {
                let Some(target) = query.block_filter_target() else {
                    query.bf_wb.wait(Duration::from_millis(200));
                    continue;
                };
                let next = query.basic_filter_hwm().ok().flatten().map_or(0, |h| h + 1);
                if next > target {
                    if let Some(p) = pass.take() {
                        rbitcoin_log::info!(
                            "blockfilter: materialize done through={target} heights={} elapsed={:.1}s",
                            target + 1 - p.from,
                            p.started.elapsed().as_secs_f64()
                        );
                    }
                    query.bf_wb.wait(Duration::from_millis(200));
                    continue;
                }
                let t0 = Instant::now();
                let res = if target - next >= SEAL_CHUNK {
                    let p = pass.get_or_insert_with(|| {
                        rbitcoin_log::info!(
                            "blockfilter: materialize from={next} to={target} workers={workers}"
                        );
                        Materialize {
                            from: next,
                            started: t0,
                            last_log: t0,
                        }
                    });
                    if p.last_log.elapsed() >= PROGRESS_EVERY {
                        let done = next - p.from;
                        let secs = p.started.elapsed().as_secs_f64();
                        let rate = f64::from(done) / secs.max(0.001);
                        rbitcoin_log::info!(
                            "blockfilter: materialize next={next} tip={target} rate={rate:.0}/s remain={} elapsed={secs:.0}s",
                            target + 1 - next
                        );
                        p.last_log = Instant::now();
                    }
                    query.seal_block_filters_parallel(target, workers, &stop)
                } else {
                    query.seal_block_filters_chunk(target)
                };
                match res {
                    Ok(0) => query.bf_wb.wait(Duration::from_millis(200)),
                    Ok(_) if pass.is_none() => {
                        rbitcoin_log::info!(
                            "blockfilter: apply h={target} wall={}ms",
                            t0.elapsed().as_millis()
                        );
                    }
                    Ok(_) => {}
                    Err(e) => {
                        rbitcoin_log::error!("blockfilter write-behind: {e}");
                        stop.store(true, Ordering::SeqCst);
                        on_fatal();
                        return;
                    }
                }
            }
        })
        .expect("spawn block filter write-behind")
}
