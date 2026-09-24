//! Sidecar for heights whose spend annotations and Class A bodies are durable.
//!
//! Not a [`SCHEMA_VERSION`](rbitcoin_primitives::SCHEMA_VERSION) bump. A missing
//! file means nothing has been synced yet.

use crate::error::StoreError;
use crate::file::write_synced_tmp_rename;
use rbitcoin_primitives::{schema_file_openable, SCHEMA_VERSION, STORE_MAGIC};
use std::path::Path;

/// `store/spend_durable`. Annotated-through and durable-through heights.
pub const SPEND_DURABLE_NAME: &str = "spend_durable";

/// Confirm batches between Class A `sync_data`. Not a knob.
pub(crate) const SPEND_DURABLE_BATCHES: u32 = 8;

/// Wall time between those syncs when batches arrive slowly. Not a knob.
pub(crate) const SPEND_DURABLE_INTERVAL_MS: u64 = 30_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SpendDurable {
    annotated_through: u32,
    durable_through: u32,
}

impl SpendDurable {
    pub(crate) fn new(annotated_through: u32, durable_through: u32) -> Self {
        Self {
            annotated_through,
            durable_through,
        }
    }

    pub(crate) fn annotated_through(self) -> u32 {
        self.annotated_through
    }

    pub(crate) fn durable_through(self) -> u32 {
        self.durable_through
    }

    pub(crate) fn load(dir: &Path) -> Result<Option<Self>, StoreError> {
        let path = dir.join(SPEND_DURABLE_NAME);
        if !path.exists() {
            return Ok(None);
        }
        let buf = std::fs::read(&path).map_err(|e| StoreError::io(&path, e))?;
        if buf.len() != 16 {
            return Err(StoreError::Corrupt("spend_durable short"));
        }
        if buf[0..4] != STORE_MAGIC {
            return Err(StoreError::BadMagic);
        }
        let ver = u16::from_le_bytes(buf[4..6].try_into().unwrap());
        if !schema_file_openable(ver) {
            return Err(StoreError::BadSchema(ver));
        }
        Ok(Some(Self {
            annotated_through: u32::from_le_bytes(buf[8..12].try_into().unwrap()),
            durable_through: u32::from_le_bytes(buf[12..16].try_into().unwrap()),
        }))
    }

    pub(crate) fn store(&self, dir: &Path) -> Result<(), StoreError> {
        let mut buf = [0u8; 16];
        buf[0..4].copy_from_slice(&STORE_MAGIC);
        buf[4..6].copy_from_slice(&SCHEMA_VERSION.to_le_bytes());
        buf[8..12].copy_from_slice(&self.annotated_through.to_le_bytes());
        buf[12..16].copy_from_slice(&self.durable_through.to_le_bytes());
        write_synced_tmp_rename(&dir.join(SPEND_DURABLE_NAME), &buf)
    }
}

pub(crate) fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// True when the write thread should `sync_data` and advance the marker.
pub(crate) fn spend_sync_due(batches: u32, elapsed_ms: u64) -> bool {
    batches >= SPEND_DURABLE_BATCHES || elapsed_ms >= SPEND_DURABLE_INTERVAL_MS
}

/// `n == 0` stays "whole chain". A missing marker keeps `n`. Otherwise the
/// window is at least six and reaches back to `durable_through`.
pub(crate) fn widen_checkblocks(n: u32, tip: Option<u32>, durable_through: Option<u32>) -> u32 {
    if n == 0 {
        return 0;
    }
    let (Some(tip), Some(d)) = (tip, durable_through) else {
        return n;
    };
    let span = tip.saturating_sub(d);
    n.max(crate::VERIFY_TIP_BLOCKS).max(span)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spend_sync_due_is_batches_or_elapsed() {
        assert!(!spend_sync_due(SPEND_DURABLE_BATCHES - 1, 0));
        assert!(spend_sync_due(SPEND_DURABLE_BATCHES, 0));
        assert!(!spend_sync_due(1, SPEND_DURABLE_INTERVAL_MS - 1));
        assert!(spend_sync_due(1, SPEND_DURABLE_INTERVAL_MS));
    }

    #[test]
    fn widen_keeps_zero_and_six_and_the_marker_span() {
        assert_eq!(widen_checkblocks(0, Some(20), Some(0)), 0);
        assert_eq!(widen_checkblocks(6, Some(9), None), 6);
        assert_eq!(widen_checkblocks(1, Some(9), Some(9)), 6);
        assert_eq!(widen_checkblocks(6, Some(9), Some(0)), 9);
        assert_eq!(widen_checkblocks(100, Some(9), Some(0)), 100);
    }

    #[test]
    fn eight_batches_publish_the_marker_and_clamp_drops_a_stale_height() {
        let dir = std::env::temp_dir().join(format!(
            "rbitcoin-spend-due-{}-{}",
            std::process::id(),
            unix_ms()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let s = crate::Store::create_tiny(&dir).unwrap();
        assert!(s.spend_annotated_through().unwrap().is_none());
        for _ in 0..(SPEND_DURABLE_BATCHES - 1) {
            assert_eq!(s.note_spend_durable_batch(4).unwrap(), 0);
        }
        assert!(s.spend_annotated_through().unwrap().is_none());
        assert!(unix_ms() > 1_700_000_000_000);
        assert!(s.note_spend_durable_batch(4).unwrap() > 0);
        assert_eq!(s.spend_annotated_through().unwrap(), Some(0));

        s.confirmed
            .set(rbitcoin_primitives::Height(0), rbitcoin_primitives::Fk(1))
            .unwrap();
        SpendDurable::new(5, 0).store(s.path()).unwrap();
        let m = SpendDurable::load(s.path()).unwrap().unwrap();
        assert_eq!((m.annotated_through(), m.durable_through()), (5, 0));
        s.clamp_spend_durable().unwrap();
        let m = SpendDurable::load(s.path()).unwrap().unwrap();
        assert_eq!((m.annotated_through(), m.durable_through()), (0, 0));
        SpendDurable::new(0, 5).store(s.path()).unwrap();
        let m = SpendDurable::load(s.path()).unwrap().unwrap();
        assert_eq!((m.annotated_through(), m.durable_through()), (0, 5));
        s.clamp_spend_durable().unwrap();
        let m = SpendDurable::load(s.path()).unwrap().unwrap();
        assert_eq!((m.annotated_through(), m.durable_through()), (0, 0));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
