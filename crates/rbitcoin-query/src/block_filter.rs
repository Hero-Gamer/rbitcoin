//! BIP158 basic filters over [`rbitcoin_store::BlockFilterTable`].
//!
//! Additive index under the store dir; Class A is unchanged. Filters are
//! built from Class A (output scripts plus spent prevout scripts through
//! `input.body` parent edges), so heights below a seqsigwit prune still build.
//! The watermark is the last committed height.

use bitcoin::bip158::{BlockFilter, FilterHash, FilterHeader, GcsFilterWriter};
use bitcoin::hashes::Hash;
use rbitcoin_primitives::{Fk, Height};
use rbitcoin_store::{
    read_index_window, BlockFilterRecord, BlockFilterSlot, BlockFilterTable, IndexHeight,
    IndexWindow, StoreError,
};

use crate::{Query, QueryError};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Condvar, Mutex};

/// BIP158 basic filter Golomb-Rice parameters (`M`, `P`).
const BASIC_FILTER_M: u64 = 784_931;
const BASIC_FILTER_P: u8 = 19;
/// Heights per build unit. Bounds RAM per in-flight chunk and how long a
/// disconnect waits on the appender lock.
const SEAL_CHUNK: u32 = 64;
/// Finished chunks committed per durable put (one fsync pair) while a gap closes.
const GROUP_CHUNKS: u32 = 8;

/// Block filter write-behind state (one appender thread).
pub(crate) struct BlockFilterWriteBehind {
    /// Serializes commits against disconnect truncate.
    appender: Mutex<()>,
    wake: Mutex<()>,
    wake_cv: Condvar,
}

impl BlockFilterWriteBehind {
    pub(crate) fn new() -> Self {
        Self {
            appender: Mutex::new(()),
            wake: Mutex::new(()),
            wake_cv: Condvar::new(),
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

/// GCS-encode a basic filter keyed by `block_hash` (internal byte order).
fn encode_basic_filter<'a>(
    block_hash: &[u8; 32],
    elements: impl Iterator<Item = &'a [u8]>,
) -> Result<BlockFilter, QueryError> {
    let k0 = u64::from_le_bytes(block_hash[0..8].try_into().unwrap());
    let k1 = u64::from_le_bytes(block_hash[8..16].try_into().unwrap());
    let mut content = Vec::new();
    let mut writer = GcsFilterWriter::new(&mut content, k0, k1, BASIC_FILTER_M, BASIC_FILTER_P);
    for e in elements {
        writer.add_element(e);
    }
    writer
        .finish()
        .map_err(|_| StoreError::Corrupt("invariant: blockfilter encode"))?;
    Ok(BlockFilter::new(&content))
}

impl Query {
    /// Basic filter of best-chain `height`, built from Class A, and its `header_fk`.
    fn build_basic_filter(&self, height: Height) -> Result<(BlockFilter, Fk), QueryError> {
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
        let filter = encode_basic_filter(&rec.hash, elements.iter().map(Vec::as_slice))?;
        Ok((filter, header_fk))
    }

    /// Confirmed heights `start..=end` as reads for [`Self::read_index_window`];
    /// heights at or above `tweaks_from` also read what tweaks need.
    pub fn index_heights(
        &self,
        start: u32,
        end: u32,
        tweaks_from: Option<u32>,
    ) -> Result<Vec<IndexHeight>, QueryError> {
        (start..=end)
            .map(|h| {
                let header_fk = self
                    .store
                    .confirmed
                    .get(Height(h))?
                    .ok_or(StoreError::Corrupt("invariant: index height not confirmed"))?;
                let (first, n) = self
                    .store
                    .header_txs
                    .get_range(header_fk)?
                    .ok_or(StoreError::Corrupt("confirmed header missing body list"))?;
                Ok(IndexHeight {
                    height: Height(h),
                    header_fk,
                    first,
                    n,
                    tweaks: tweaks_from.is_some_and(|t| h >= t),
                })
            })
            .collect()
    }

    /// Read a window of blocks and the parents they spend (completion session).
    pub fn read_index_window(&self, heights: &[IndexHeight]) -> Result<IndexWindow, QueryError> {
        read_index_window(&self.store.txs, heights)
    }

    /// Basic filter of `window.blocks[i]`.
    pub fn basic_filter_from_window(
        &self,
        window: &IndexWindow,
        i: usize,
    ) -> Result<BlockFilter, QueryError> {
        const OP_RETURN: u8 = 0x6a;
        let block = &window.blocks[i];
        let hash = self.store.get_header(block.header_fk)?.hash;
        let mut elements: Vec<&[u8]> = Vec::new();
        for tx in &block.txs {
            for o in &tx.outs {
                if o.script.first() != Some(&OP_RETURN) {
                    elements.push(&o.script);
                }
            }
        }
        for e in block.edges.iter().flatten().filter(|e| !e.parent.is_null()) {
            let out = window.prevout(e.parent, e.vout).ok_or(StoreError::Corrupt(
                "invariant: blockfilter prevout missing",
            ))?;
            elements.push(&out.script);
        }
        encode_basic_filter(&hash, elements.into_iter())
    }

    pub fn block_filter_enabled(&self) -> bool {
        self.block_filter_enabled.load(Ordering::SeqCst)
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
        self.block_filter_enabled.store(enabled, Ordering::SeqCst);
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

    /// Heights the tip leads the committed filter watermark (0 when off).
    pub fn block_filter_lag_heights(&self) -> u32 {
        let Some(table) = self.block_filter_table() else {
            return 0;
        };
        match self.tip_height() {
            None => 0,
            Some(tip) => tip
                .0
                .saturating_add(1)
                .saturating_sub(table.next_height().0),
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

    /// Filter bytes for `start..=end` in one idx and one body read per
    /// segment. `None` when the index is off or `end` is past the watermark.
    pub fn basic_filters(&self, start: u32, end: u32) -> Result<Option<Vec<Vec<u8>>>, QueryError> {
        let Some(table) = self.block_filter_table() else {
            return Ok(None);
        };
        Ok(table
            .filters(Height(start), Height(end))?
            .map(|v| v.into_iter().map(|(body, _)| body).collect()))
    }

    /// Filter hash and header for `start..=end` without filter bytes.
    /// `None` when the index is off or `end` is past the watermark.
    pub fn basic_filter_hashes_and_headers(
        &self,
        start: u32,
        end: u32,
    ) -> Result<Option<Vec<(FilterHash, FilterHeader)>>, QueryError> {
        let Some(table) = self.block_filter_table() else {
            return Ok(None);
        };
        Ok(table.slots(Height(start), Height(end))?.map(|v| {
            v.into_iter()
                .map(|s| {
                    (
                        FilterHash::from_byte_array(s.filter_hash),
                        FilterHeader::from_byte_array(s.filter_header),
                    )
                })
                .collect()
        }))
    }

    /// Heights index write-behind may seal now: released through tip.
    pub fn index_target(&self) -> Option<u32> {
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

    /// Seal `[next, through]`; returns heights committed.
    ///
    /// A one-chunk gap (the tip) builds on this thread. A wider gap (the
    /// materialize, or a later catch-up) runs up to `workers` threads for the
    /// whole pass: they take 64-height chunks in order from a shared cursor
    /// and this thread commits them in order, up to `GROUP_CHUNKS` finished
    /// chunks per durable put. `stop` is checked between commits. A commit
    /// that finds the watermark or `confirmed[h]` moved (a reorg) ends the
    /// pass early. `on_commit` gets the new watermark after each commit.
    ///
    /// Trade: at most `2 × workers` built chunks wait in RAM (tens of MB at
    /// mainnet sizes), and `workers` threads read Class A at once, only
    /// while a gap wider than one chunk is closing.
    fn seal_block_filters(
        &self,
        through: u32,
        workers: u32,
        stop: &AtomicBool,
        on_commit: &mut dyn FnMut(u32),
    ) -> Result<u32, QueryError> {
        let Some(table) = self.block_filter_table() else {
            return Ok(0);
        };
        let start = table.next_height().0;
        if start > through {
            return Ok(0);
        }
        let t0 = std::time::Instant::now();
        let n_chunks = (through - start) / SEAL_CHUNK + 1;
        let chunk = |k: u32| {
            let s = start + k * SEAL_CHUNK;
            (s, through.min(s + SEAL_CHUNK - 1))
        };
        let done = if n_chunks == 1 || workers <= 1 {
            let mut done = 0;
            for k in 0..n_chunks {
                if k > 0 && stop.load(Ordering::Relaxed) {
                    break;
                }
                let (s, e) = chunk(k);
                let n = self.commit_basic_filters(table, s, &self.build_basic_filters(s, e)?)?;
                done += n;
                if n == 0 {
                    break;
                }
                on_commit(s + n - 1);
            }
            done
        } else {
            self.seal_block_filters_pipelined(table, n_chunks, workers, &chunk, stop, on_commit)?
        };
        crate::note_confirm(
            &self.confirm_stats().blockfilter_ns,
            t0.elapsed().as_nanos() as u64,
        );
        Ok(done)
    }

    fn seal_block_filters_pipelined(
        &self,
        table: &BlockFilterTable,
        n_chunks: u32,
        workers: u32,
        chunk: &(dyn Fn(u32) -> (u32, u32) + Sync),
        stop: &AtomicBool,
        on_commit: &mut dyn FnMut(u32),
    ) -> Result<u32, QueryError> {
        type Built = Result<Vec<(BlockFilter, Fk)>, QueryError>;
        let window = 2 * workers;
        let cursor = AtomicU32::new(0);
        let committed = Mutex::new(0u32);
        let advanced = Condvar::new();
        let abort = AtomicBool::new(false);
        let (tx, rx) = std::sync::mpsc::channel::<(u32, Built)>();
        std::thread::scope(|scope| {
            for _ in 0..workers.min(n_chunks) {
                let tx = tx.clone();
                let (cursor, committed, advanced, abort) = (&cursor, &committed, &advanced, &abort);
                scope.spawn(move || loop {
                    let k = cursor.fetch_add(1, Ordering::Relaxed);
                    if k >= n_chunks {
                        return;
                    }
                    let mut c = committed.lock().unwrap_or_else(|e| e.into_inner());
                    while k >= *c + window && !abort.load(Ordering::Relaxed) {
                        c = advanced.wait(c).unwrap_or_else(|e| e.into_inner());
                    }
                    drop(c);
                    if abort.load(Ordering::Relaxed) {
                        return;
                    }
                    let (s, e) = chunk(k);
                    if tx.send((k, self.build_basic_filters(s, e))).is_err() {
                        return;
                    }
                });
            }
            drop(tx);
            let res = (|| {
                let mut ready: BTreeMap<u32, Built> = BTreeMap::new();
                let (mut next_k, mut done) = (0u32, 0u32);
                while next_k < n_chunks && !stop.load(Ordering::Relaxed) {
                    let Ok((k, built)) = rx.recv() else { break };
                    ready.insert(k, built);
                    let mut group = Vec::new();
                    let mut end_k = next_k;
                    while end_k - next_k < GROUP_CHUNKS {
                        let Some(built) = ready.remove(&end_k) else {
                            break;
                        };
                        group.extend(built?);
                        end_k += 1;
                    }
                    if group.is_empty() {
                        continue;
                    }
                    let s = chunk(next_k).0;
                    let n = self.commit_basic_filters(table, s, &group)?;
                    done += n;
                    if n == 0 {
                        break;
                    }
                    next_k = end_k;
                    *committed.lock().unwrap_or_else(|e| e.into_inner()) = next_k;
                    advanced.notify_all();
                    on_commit(s + n - 1);
                }
                Ok(done)
            })();
            abort.store(true, Ordering::Relaxed);
            advanced.notify_all();
            drop(rx);
            res
        })
    }

    /// Seal every released height now (regtest `generate`, tests).
    pub fn seal_block_filters_released(&self) -> Result<(), QueryError> {
        let never = AtomicBool::new(false);
        while let Some(t) = self.index_target() {
            if self.seal_block_filters(t, 1, &never, &mut |_| {})? == 0 {
                break;
            }
        }
        Ok(())
    }

    /// Next filter height to seal (`None` when the filter index is off).
    pub fn filter_index_next(&self) -> Option<u32> {
        self.block_filter_table().map(|t| t.next_height().0)
    }

    /// Commit filters built for `start..` (a window). Returns heights
    /// committed; 0 when the watermark or a `confirmed[h]` moved (a reorg).
    pub fn commit_window_filters(
        &self,
        start: u32,
        built: &[(BlockFilter, Fk)],
    ) -> Result<u32, QueryError> {
        let Some(table) = self.block_filter_table() else {
            return Ok(0);
        };
        self.commit_basic_filters(table, start, built)
    }

    /// Wait for an index release (or `d`).
    pub fn wait_index_release(&self, d: std::time::Duration) {
        self.bf_wb.wait(d);
    }

    pub fn truncate_basic_filters_to_tip(&self) -> Result<(), QueryError> {
        match self.block_filters.get() {
            Some(t) => Ok(t.truncate_through(self.tip_height())?),
            None => Ok(()),
        }
    }
}

/// The block filter appender: after catch-up, seals `(hwm, released tip]`
/// off the confirm path. A gap wider than one chunk is a materialize
/// (worker pool, progress logs); at the tip each release is one chunk.
///
/// Stop is checked between commits, so shutdown waits at most one chunk
/// build. Apply errors request `stop` and `on_fatal`, like the scripthash
/// appender. `on_caught_up` runs once, the first time the committed filters
/// reach the released tip (the node starts advertising `NODE_COMPACT_FILTERS`).
pub fn spawn_block_filter_writebehind(
    query: std::sync::Arc<Query>,
    stop: std::sync::Arc<AtomicBool>,
    on_fatal: impl FnOnce() + Send + 'static,
    on_caught_up: impl FnOnce() + Send + 'static,
) -> std::thread::JoinHandle<()> {
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
            let mut on_caught_up = Some(on_caught_up);
            while !stop.load(Ordering::Relaxed) {
                let next = query.basic_filter_hwm().ok().flatten().map_or(0, |h| h + 1);
                let released = query.index_target();
                if released.is_some_and(|t| next > t) {
                    if let Some(f) = on_caught_up.take() {
                        rbitcoin_log::info!(
                            "blockfilter: caught up through={}; advertising NODE_COMPACT_FILTERS",
                            next - 1
                        );
                        f();
                    }
                }
                let Some(target) = released.filter(|&t| t >= next) else {
                    query.bf_wb.wait(Duration::from_millis(200));
                    continue;
                };
                let t0 = Instant::now();
                let wide = target - next >= SEAL_CHUNK;
                let res = if wide {
                    rbitcoin_log::info!(
                        "blockfilter: materialize from={next} to={target} workers={workers}"
                    );
                    let mut last_log = t0;
                    query.seal_block_filters(target, workers, &stop, &mut |hwm| {
                        if last_log.elapsed() >= PROGRESS_EVERY {
                            let secs = t0.elapsed().as_secs_f64();
                            rbitcoin_log::info!(
                                "blockfilter: materialize next={} tip={target} rate={:.0}/s remain={} elapsed={secs:.0}s",
                                hwm + 1,
                                f64::from(hwm + 1 - next) / secs.max(0.001),
                                target - hwm
                            );
                            last_log = Instant::now();
                        }
                    })
                } else {
                    query.seal_block_filters(target, 1, &stop, &mut |_| {})
                };
                match res {
                    Err(e) => {
                        rbitcoin_log::error!("blockfilter write-behind: {e}");
                        stop.store(true, Ordering::SeqCst);
                        on_fatal();
                        return;
                    }
                    Ok(0) => query.bf_wb.wait(Duration::from_millis(200)),
                    Ok(n) if wide => rbitcoin_log::info!(
                        "blockfilter: materialize {} through={} heights={n} elapsed={:.1}s",
                        if next + n > target { "done" } else { "stopped" },
                        next + n - 1,
                        t0.elapsed().as_secs_f64()
                    ),
                    Ok(_) => rbitcoin_log::info!(
                        "blockfilter: apply h={target} wall={}ms",
                        t0.elapsed().as_millis()
                    ),
                }
            }
        })
        .expect("spawn block filter write-behind")
}
