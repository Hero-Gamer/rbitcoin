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
use std::sync::atomic::Ordering;
use std::sync::{Condvar, Mutex};

/// BIP158 basic filter Golomb-Rice parameters (`M`, `P`).
const BASIC_FILTER_M: u64 = 784_931;
const BASIC_FILTER_P: u8 = 19;
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
