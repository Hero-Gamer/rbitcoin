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

    /// Seal the next chunk of `[next, through]`. Returns heights committed.
    ///
    /// Builds without the appender lock, then commits under it only if the
    /// watermark is still `next` and every built height is still
    /// `confirmed[h]` (a disconnect in between drops the chunk).
    pub fn seal_block_filters_chunk(&self, through: u32) -> Result<u32, QueryError> {
        let Some(table) = self.block_filter_table() else {
            return Ok(0);
        };
        let start = table.next_height().0;
        if start > through {
            return Ok(0);
        }
        let end = through.min(start.saturating_add(SEAL_CHUNK - 1));
        let built = (start..=end)
            .map(|h| self.build_basic_filter(Height(h)))
            .collect::<Result<Vec<_>, _>>()?;

        let _appender = self.bf_wb.lock_appender();
        if table.next_height().0 != start {
            return Ok(0);
        }
        for (h, (_, header_fk)) in (start..=end).zip(&built) {
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
        let mut slots = Vec::with_capacity(built.len());
        for (filter, header_fk) in &built {
            let header = filter.filter_header(&prev);
            slots.push(BlockFilterSlot {
                header_fk: *header_fk,
                filter_hash: FilterHash::hash(&filter.content).to_byte_array(),
                filter_header: header.to_byte_array(),
            });
            prev = header;
        }
        let recs: Vec<BlockFilterRecord<'_>> = (start..=end)
            .zip(&built)
            .zip(&slots)
            .map(|((h, (filter, _)), slot)| BlockFilterRecord {
                height: Height(h),
                slot: *slot,
                filter: &filter.content,
            })
            .collect();
        table.put(&recs)?;
        Ok(end - start + 1)
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

/// The block filter appender: after catch-up, seals `(hwm, released tip]`
/// off the confirm path. The first pass over a large gap is the materialize;
/// after that each released tip is one chunk.
///
/// Apply errors request `stop` and `on_fatal`, like the scripthash appender.
pub fn spawn_block_filter_writebehind(
    query: std::sync::Arc<Query>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    on_fatal: impl FnOnce() + Send + 'static,
) -> std::thread::JoinHandle<()> {
    use std::sync::atomic::Ordering;
    std::thread::Builder::new()
        .name("rbtc-bf-wb".into())
        .spawn(move || {
            // Spawned after catch-up: the current tip was already announced.
            if let Some(tip) = query.tip_height() {
                query.release_index_writebehind(tip);
            }
            while !stop.load(Ordering::Relaxed) {
                let Some(target) = query.block_filter_target() else {
                    query.bf_wb.wait(std::time::Duration::from_millis(200));
                    continue;
                };
                let t0 = std::time::Instant::now();
                match query.seal_block_filters_chunk(target) {
                    Ok(0) => query.bf_wb.wait(std::time::Duration::from_millis(200)),
                    Ok(_) => {
                        let hwm = query.basic_filter_hwm().ok().flatten();
                        rbitcoin_log::info!(
                            "blockfilter: apply h={hwm:?} wall={}ms target={target}",
                            t0.elapsed().as_millis()
                        );
                    }
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
