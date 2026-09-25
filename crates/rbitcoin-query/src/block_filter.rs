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

    /// Seal missing heights `(hwm, tip]`. No-op when the index flag is off.
    pub fn backfill_block_filters(&self) -> Result<(), QueryError> {
        let Some(tip) = self.tip_height() else {
            return Ok(());
        };
        self.backfill_block_filters_through(tip.0)
    }

    pub fn backfill_block_filters_through(&self, through: u32) -> Result<(), QueryError> {
        let Some(table) = self.block_filter_table() else {
            return Ok(());
        };
        let start = table.next_height().0;
        if start > through {
            return Ok(());
        }
        let t0 = std::time::Instant::now();
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
        for h in start..=through {
            let (filter, header_fk) = self.build_basic_filter(Height(h))?;
            let header = filter.filter_header(&prev);
            table.put(&[BlockFilterRecord {
                height: Height(h),
                slot: BlockFilterSlot {
                    header_fk,
                    filter_hash: FilterHash::hash(&filter.content).to_byte_array(),
                    filter_header: header.to_byte_array(),
                },
                filter: &filter.content,
            }])?;
            prev = header;
        }
        rbitcoin_log::debug!(
            "ibd: perf blockfilter backfill {start}..={through} us={}",
            t0.elapsed().as_micros()
        );
        Ok(())
    }

    pub fn truncate_basic_filters_to_tip(&self) -> Result<(), QueryError> {
        match self.block_filters.get() {
            Some(t) => Ok(t.truncate_through(self.tip_height())?),
            None => Ok(()),
        }
    }
}
