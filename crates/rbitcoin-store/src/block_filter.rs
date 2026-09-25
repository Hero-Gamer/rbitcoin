//! BIP158 basic filter index (`blockfilter.idx/` + `blockfilter.body/`).
//!
//! Additive side table: Class A is unchanged and a missing dir is "not built".
//! Slots are dense from height 0 on the best chain. A reorg truncates above
//! the new tip and those heights are written again.
//!
//! One fixed idx slot per height, so any height's filter, hash, or header is
//! one idx pread (plus one body pread for the filter bytes). The slot keeps
//! `header_fk` so open can drop slots a crash left on a stale branch.
//!
//! ```text
//! blockfilter.idx/meta      fmt:u32=1
//! blockfilter.idx/NNNNNN    slot[i] = off:u32 ‖ 0:u32 ‖ header_fk:u64
//!                                     ‖ filter_hash:[u8;32] ‖ filter_header:[u8;32]
//! blockfilter.body/NNNNNN   filter bytes (Core `BlockFilter` content), no prefix
//! ```
//!
//! A record ends at the next slot's `off`, or at the body logical end for the
//! last slot. When the next record's **start** would exceed `u32::MAX`, a new
//! `NNNNNN` pair starts (the sp_tweaks rule).
//!
//! **Commit order:** body pwrite → body `sync_data` → idx pwrite → idx
//! `sync_data`. An idx slot on disk therefore never points at unsynced body
//! bytes. The idx HWM may still cover a torn slot after power loss; open drops
//! trailing slots whose `off` runs backwards or past the body end, and the
//! caller checks `header_fk` against `confirmed[]`.

use crate::error::StoreError;
use crate::file::{TableFile, FILE_HEADER_LEN};
use rbitcoin_primitives::{Fk, Height, TableKind};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const SLOT: u64 = 80;
const IDX_FMT: u32 = 1;

/// Per-height idx facts (no filter bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockFilterSlot {
    pub header_fk: Fk,
    pub filter_hash: [u8; 32],
    pub filter_header: [u8; 32],
}

/// One height to append.
pub struct BlockFilterRecord<'a> {
    pub height: Height,
    pub slot: BlockFilterSlot,
    pub filter: &'a [u8],
}

struct Seg {
    file_id: u32,
    first_slot: u64,
    n_slots: u64,
    idx: TableFile,
    body: TableFile,
}

struct Inner {
    segs: Vec<Seg>,
}

/// Height-dense basic filter table.
pub struct BlockFilterTable {
    dir: PathBuf,
    inner: Mutex<Inner>,
}

fn encode_slot(off: u32, s: &BlockFilterSlot) -> [u8; SLOT as usize] {
    let mut b = [0u8; SLOT as usize];
    b[0..4].copy_from_slice(&off.to_le_bytes());
    b[8..16].copy_from_slice(&s.header_fk.0.to_le_bytes());
    b[16..48].copy_from_slice(&s.filter_hash);
    b[48..80].copy_from_slice(&s.filter_header);
    b
}

fn decode_slot(b: &[u8]) -> (u32, BlockFilterSlot) {
    let off = u32::from_le_bytes(b[0..4].try_into().unwrap());
    let header_fk = Fk(u64::from_le_bytes(b[8..16].try_into().unwrap()));
    let mut filter_hash = [0u8; 32];
    filter_hash.copy_from_slice(&b[16..48]);
    let mut filter_header = [0u8; 32];
    filter_header.copy_from_slice(&b[48..80]);
    (
        off,
        BlockFilterSlot {
            header_fk,
            filter_hash,
            filter_header,
        },
    )
}

impl BlockFilterTable {
    fn idx_dir(dir: &Path) -> PathBuf {
        dir.join("blockfilter.idx")
    }

    fn body_dir(dir: &Path) -> PathBuf {
        dir.join("blockfilter.body")
    }

    fn meta_path(dir: &Path) -> PathBuf {
        Self::idx_dir(dir).join("meta")
    }

    fn seg_idx_path(dir: &Path, file_id: u32) -> PathBuf {
        Self::idx_dir(dir).join(format!("{file_id:06}"))
    }

    fn seg_body_path(dir: &Path, file_id: u32) -> PathBuf {
        Self::body_dir(dir).join(format!("{file_id:06}"))
    }

    fn create_seg(dir: &Path, file_id: u32, first_slot: u64) -> Result<Seg, StoreError> {
        let idx = TableFile::create(Self::seg_idx_path(dir, file_id), TableKind::ArrayLink)?;
        idx.set_grow_tight(true);
        let body = TableFile::create(Self::seg_body_path(dir, file_id), TableKind::BlockFilter)?;
        Ok(Seg {
            file_id,
            first_slot,
            n_slots: 0,
            idx,
            body,
        })
    }

    fn create(dir: &Path) -> Result<Self, StoreError> {
        std::fs::create_dir_all(Self::idx_dir(dir)).map_err(|e| StoreError::io(dir, e))?;
        std::fs::create_dir_all(Self::body_dir(dir)).map_err(|e| StoreError::io(dir, e))?;
        let meta = TableFile::create(Self::meta_path(dir), TableKind::ArrayLink)?;
        meta.write_at_pwrite(FILE_HEADER_LEN as u64, &IDX_FMT.to_le_bytes())?;
        meta.flush()?;
        let seg = Self::create_seg(dir, 0, 0)?;
        Ok(Self {
            dir: dir.to_path_buf(),
            inner: Mutex::new(Inner { segs: vec![seg] }),
        })
    }

    fn open(dir: &Path) -> Result<Self, StoreError> {
        let meta = TableFile::open(Self::meta_path(dir), TableKind::ArrayLink)?;
        if meta.logical_len() < FILE_HEADER_LEN as u64 + 4 {
            return Err(StoreError::Corrupt("blockfilter.idx/meta short"));
        }
        let mut fmt = [0u8; 4];
        meta.read_at(FILE_HEADER_LEN as u64, &mut fmt)?;
        if u32::from_le_bytes(fmt) != IDX_FMT {
            return Err(StoreError::Corrupt("blockfilter idx format"));
        }
        let mut segs = Vec::new();
        let mut first_slot = 0u64;
        for file_id in 0u32.. {
            let ip = Self::seg_idx_path(dir, file_id);
            let bp = Self::seg_body_path(dir, file_id);
            if !ip.exists() && !bp.exists() {
                break;
            }
            if !ip.exists() || !bp.exists() {
                return Err(StoreError::Corrupt("blockfilter incomplete segment"));
            }
            let idx = TableFile::open(&ip, TableKind::ArrayLink)?;
            idx.set_grow_tight(true);
            let body = TableFile::open(&bp, TableKind::BlockFilter)?;
            let extra = idx.logical_len().saturating_sub(FILE_HEADER_LEN as u64);
            let mut seg = Seg {
                file_id,
                first_slot,
                n_slots: extra / SLOT,
                idx,
                body,
            };
            Self::clamp_torn_tail(&mut seg)?;
            first_slot = first_slot.saturating_add(seg.n_slots);
            segs.push(seg);
        }
        if segs.is_empty() {
            return Err(StoreError::Corrupt("blockfilter no segments"));
        }
        Ok(Self {
            dir: dir.to_path_buf(),
            inner: Mutex::new(Inner { segs }),
        })
    }

    /// Keep the longest slot prefix whose offsets run forward inside the body.
    fn clamp_torn_tail(seg: &mut Seg) -> Result<(), StoreError> {
        if seg.n_slots == 0 {
            return seg.idx.set_logical_len(FILE_HEADER_LEN as u64);
        }
        let mut buf = vec![0u8; (seg.n_slots * SLOT) as usize];
        seg.idx.read_at(FILE_HEADER_LEN as u64, &mut buf)?;
        let body_end = seg.body.logical_len();
        let mut prev = FILE_HEADER_LEN as u64;
        let mut keep = 0u64;
        for i in 0..seg.n_slots {
            let s = (i * SLOT) as usize;
            let (off, _) = decode_slot(&buf[s..s + SLOT as usize]);
            let off = u64::from(off);
            if off < prev || off > body_end {
                break;
            }
            prev = off;
            keep = i + 1;
        }
        let idx_len = FILE_HEADER_LEN as u64 + keep * SLOT;
        if keep < seg.n_slots || seg.idx.logical_len() != idx_len {
            rbitcoin_log::warn!(
                "blockfilter: dropping torn idx tail file={:06} slots {}→{}",
                seg.file_id,
                seg.n_slots,
                keep
            );
            seg.idx.set_logical_len(idx_len)?;
            seg.idx.flush()?;
            seg.n_slots = keep;
        }
        Ok(())
    }

    /// Open the table under `dir`, or create an empty one.
    pub fn open_or_create(dir: impl AsRef<Path>) -> Result<Self, StoreError> {
        let dir = dir.as_ref();
        if Self::meta_path(dir).is_file() {
            return Self::open(dir);
        }
        let _ = std::fs::remove_dir_all(Self::idx_dir(dir));
        let _ = std::fs::remove_dir_all(Self::body_dir(dir));
        Self::create(dir)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Next height this table accepts (0 when empty).
    pub fn next_height(&self) -> Height {
        Height(self.lock().segs.iter().map(|s| s.n_slots).sum::<u64>() as u32)
    }

    fn locate(inner: &Inner, height: Height) -> Option<(usize, u64)> {
        let g = u64::from(height.0);
        inner
            .segs
            .iter()
            .enumerate()
            .find(|(_, s)| g >= s.first_slot && g < s.first_slot.saturating_add(s.n_slots))
            .map(|(si, s)| (si, g - s.first_slot))
    }

    fn read_slot_at(seg: &Seg, local: u64) -> Result<(u32, BlockFilterSlot), StoreError> {
        let mut b = [0u8; SLOT as usize];
        seg.idx
            .read_at(FILE_HEADER_LEN as u64 + local * SLOT, &mut b)?;
        Ok(decode_slot(&b))
    }

    /// Idx facts at `height` (one pread). `None` past the watermark.
    pub fn slot(&self, height: Height) -> Result<Option<BlockFilterSlot>, StoreError> {
        let inner = self.lock();
        let Some((si, local)) = Self::locate(&inner, height) else {
            return Ok(None);
        };
        Ok(Some(Self::read_slot_at(&inner.segs[si], local)?.1))
    }

    /// Filter bytes and idx facts at `height`. `None` past the watermark.
    pub fn filter(&self, height: Height) -> Result<Option<(Vec<u8>, BlockFilterSlot)>, StoreError> {
        let inner = self.lock();
        let Some((si, local)) = Self::locate(&inner, height) else {
            return Ok(None);
        };
        let seg = &inner.segs[si];
        let need_next = local + 1 < seg.n_slots;
        let mut b = vec![0u8; (SLOT * (1 + u64::from(need_next))) as usize];
        seg.idx
            .read_at(FILE_HEADER_LEN as u64 + local * SLOT, &mut b)?;
        let (start, slot) = decode_slot(&b[..SLOT as usize]);
        let end = if need_next {
            u64::from(decode_slot(&b[SLOT as usize..]).0)
        } else {
            seg.body.logical_len()
        };
        let start = u64::from(start);
        if end < start || start < FILE_HEADER_LEN as u64 {
            return Err(StoreError::Corrupt("invariant: blockfilter off order"));
        }
        let mut body = vec![0u8; (end - start) as usize];
        seg.body.read_at(start, &mut body)?;
        Ok(Some((body, slot)))
    }

    fn roll(dir: &Path, inner: &mut Inner) -> Result<(), StoreError> {
        let first_slot = inner.segs.iter().map(|s| s.n_slots).sum();
        let file_id = u32::try_from(inner.segs.len())
            .map_err(|_| StoreError::Corrupt("blockfilter too many segments"))?;
        inner.segs.push(Self::create_seg(dir, file_id, first_slot)?);
        Ok(())
    }

    /// Append consecutive heights starting at [`Self::next_height`], durably.
    pub fn put(&self, items: &[BlockFilterRecord<'_>]) -> Result<(), StoreError> {
        let mut inner = self.lock();
        let next = inner.segs.iter().map(|s| s.n_slots).sum::<u64>();
        if items
            .iter()
            .zip(next..)
            .any(|(it, want)| u64::from(it.height.0) != want)
        {
            return Err(StoreError::Corrupt(
                "invariant: blockfilter put not next height",
            ));
        }
        let mut i = 0usize;
        while i < items.len() {
            let tail_end = inner
                .segs
                .last()
                .map(|s| s.body.logical_len())
                .unwrap_or(FILE_HEADER_LEN as u64);
            if tail_end > u64::from(u32::MAX) {
                Self::roll(&self.dir, &mut inner)?;
            }
            let tail = inner
                .segs
                .last_mut()
                .ok_or(StoreError::Corrupt("blockfilter no segments"))?;
            let start = tail.body.logical_len();
            let mut body = Vec::new();
            let mut idx_blob = Vec::new();
            while i < items.len() {
                let rec_start = start + body.len() as u64;
                let Ok(off) = u32::try_from(rec_start) else {
                    break;
                };
                idx_blob.extend_from_slice(&encode_slot(off, &items[i].slot));
                body.extend_from_slice(items[i].filter);
                i += 1;
            }
            if idx_blob.is_empty() {
                return Err(StoreError::Corrupt("blockfilter body exceeds u32 off"));
            }
            tail.body.write_at_pwrite(start, &body)?;
            tail.body.flush()?;
            tail.idx
                .write_at_pwrite(FILE_HEADER_LEN as u64 + tail.n_slots * SLOT, &idx_blob)?;
            tail.idx.flush()?;
            tail.n_slots += idx_blob.len() as u64 / SLOT;
        }
        Ok(())
    }

    /// Drop heights above `tip` (`None` drops every slot).
    pub fn truncate_through(&self, tip: Option<Height>) -> Result<(), StoreError> {
        let keep = tip.map(|h| u64::from(h.0) + 1).unwrap_or(0);
        let mut inner = self.lock();
        let total: u64 = inner.segs.iter().map(|s| s.n_slots).sum();
        if keep >= total {
            return Ok(());
        }
        let si = if keep == 0 {
            0
        } else {
            let last = keep - 1;
            inner
                .segs
                .iter()
                .position(|s| last >= s.first_slot && last < s.first_slot + s.n_slots)
                .ok_or(StoreError::Corrupt("blockfilter truncate locate"))?
        };
        let local_keep = keep - inner.segs[si].first_slot;
        let seg = &mut inner.segs[si];
        if local_keep < seg.n_slots {
            let body_end = u64::from(Self::read_slot_at(seg, local_keep)?.0);
            seg.idx
                .set_logical_len(FILE_HEADER_LEN as u64 + local_keep * SLOT)?;
            seg.idx.flush()?;
            seg.body
                .set_logical_len(body_end.max(FILE_HEADER_LEN as u64))?;
            seg.body.flush()?;
            seg.n_slots = local_keep;
        }
        let drop: Vec<u32> = inner.segs.iter().skip(si + 1).map(|s| s.file_id).collect();
        inner.segs.truncate(si + 1);
        for id in drop {
            let _ = std::fs::remove_file(Self::seg_idx_path(&self.dir, id));
            let _ = std::fs::remove_file(Self::seg_body_path(&self.dir, id));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{Read, Seek, SeekFrom, Write};

    fn tmp_dir() -> crate::testutil::TempDir {
        crate::testutil::TempDir::labeled("blockfilter").unwrap()
    }

    fn slot(h: u32) -> BlockFilterSlot {
        BlockFilterSlot {
            header_fk: Fk(u64::from(h) + 100),
            filter_hash: [h as u8; 32],
            filter_header: [h as u8 ^ 0xff; 32],
        }
    }

    fn put_range(t: &BlockFilterTable, from: u32, to: u32) {
        let bodies: Vec<Vec<u8>> = (from..=to).map(|h| vec![h as u8; 3 + h as usize]).collect();
        let recs: Vec<BlockFilterRecord<'_>> = (from..=to)
            .zip(&bodies)
            .map(|(h, b)| BlockFilterRecord {
                height: Height(h),
                slot: slot(h),
                filter: b,
            })
            .collect();
        t.put(&recs).unwrap();
    }

    fn set_file_hwm(path: &Path, hwm: u64) {
        let mut f = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        f.set_len(hwm.max(FILE_HEADER_LEN as u64)).unwrap();
        let mut hdr = [0u8; FILE_HEADER_LEN];
        f.seek(SeekFrom::Start(0)).unwrap();
        f.read_exact(&mut hdr).unwrap();
        hdr[8..16].copy_from_slice(&hwm.to_le_bytes());
        f.seek(SeekFrom::Start(0)).unwrap();
        f.write_all(&hdr).unwrap();
    }

    #[test]
    fn filter_at_height_reads_only_its_record_and_survives_reopen() {
        let dir = tmp_dir();
        let t = BlockFilterTable::open_or_create(&dir).unwrap();
        assert_eq!(t.next_height(), Height(0));
        assert!(t.filter(Height(0)).unwrap().is_none());
        put_range(&t, 0, 9);
        assert_eq!(t.next_height(), Height(10));
        drop(t);

        // Scribble over every other record's body bytes: a lookup must not
        // walk earlier records to find its own.
        let body = BlockFilterTable::seg_body_path(&dir, 0);
        let mut raw = fs::read(&body).unwrap();
        let start7 = FILE_HEADER_LEN + (0..7).map(|h| 3 + h).sum::<usize>();
        let len7 = 3 + 7;
        for (i, b) in raw.iter_mut().enumerate().skip(FILE_HEADER_LEN) {
            if !(start7..start7 + len7).contains(&i) {
                *b = 0xee;
            }
        }
        fs::write(&body, &raw).unwrap();

        let t = BlockFilterTable::open_or_create(&dir).unwrap();
        assert_eq!(t.next_height(), Height(10));
        let (bytes, s) = t.filter(Height(7)).unwrap().unwrap();
        assert_eq!(bytes, vec![7u8; 10]);
        assert_eq!(s, slot(7));
        assert_eq!(t.slot(Height(9)).unwrap(), Some(slot(9)));
        assert!(t.slot(Height(10)).unwrap().is_none());
    }

    #[test]
    fn open_drops_idx_slots_past_the_body_end() {
        let dir = tmp_dir();
        let t = BlockFilterTable::open_or_create(&dir).unwrap();
        put_range(&t, 0, 4);
        drop(t);
        // Idx HWM covers slot 4 but the body stops inside record 3.
        let body = BlockFilterTable::seg_body_path(&dir, 0);
        let end3 = FILE_HEADER_LEN as u64 + (0..3).map(|h| 3 + h).sum::<u64>();
        set_file_hwm(&body, end3 + 1);

        let t = BlockFilterTable::open_or_create(&dir).unwrap();
        assert_eq!(
            t.next_height(),
            Height(4),
            "slot 4 starts past the body end"
        );
        assert_eq!(t.filter(Height(2)).unwrap().unwrap().0, vec![2u8; 5]);
        put_range(&t, 4, 4);
        assert_eq!(t.filter(Height(4)).unwrap().unwrap().0, vec![4u8; 7]);
    }

    #[test]
    fn put_must_be_next_height() {
        let dir = tmp_dir();
        let t = BlockFilterTable::open_or_create(&dir).unwrap();
        let err = t
            .put(&[BlockFilterRecord {
                height: Height(1),
                slot: slot(1),
                filter: &[1],
            }])
            .unwrap_err();
        assert!(matches!(err, StoreError::Corrupt(m) if m.contains("next height")));
    }

    #[test]
    fn truncate_then_rewrite_and_reopen() {
        let dir = tmp_dir();
        let t = BlockFilterTable::open_or_create(&dir).unwrap();
        put_range(&t, 0, 5);
        t.truncate_through(Some(Height(2))).unwrap();
        assert_eq!(t.next_height(), Height(3));
        assert!(t.filter(Height(3)).unwrap().is_none());
        put_range(&t, 3, 3);
        drop(t);
        let t = BlockFilterTable::open_or_create(&dir).unwrap();
        assert_eq!(t.next_height(), Height(4));
        assert_eq!(t.filter(Height(3)).unwrap().unwrap().0, vec![3u8; 6]);
        assert_eq!(t.filter(Height(2)).unwrap().unwrap().0, vec![2u8; 5]);
        t.truncate_through(None).unwrap();
        assert_eq!(t.next_height(), Height(0));
    }

    #[test]
    fn rolls_a_segment_when_the_next_start_passes_u32() {
        let dir = tmp_dir();
        let t = BlockFilterTable::open_or_create(&dir).unwrap();
        put_range(&t, 0, 0);
        drop(t);
        // Record 0 now appears to end one past u32::MAX (sparse file).
        let body = BlockFilterTable::seg_body_path(&dir, 0);
        set_file_hwm(&body, u64::from(u32::MAX) + 1);
        let t = BlockFilterTable::open_or_create(&dir).unwrap();
        put_range(&t, 1, 2);
        assert!(BlockFilterTable::seg_body_path(&dir, 1).is_file());
        assert_eq!(t.filter(Height(1)).unwrap().unwrap().0, vec![1u8; 4]);
        assert_eq!(t.filter(Height(2)).unwrap().unwrap().0, vec![2u8; 5]);
        t.truncate_through(Some(Height(0))).unwrap();
        assert!(!BlockFilterTable::seg_body_path(&dir, 1).exists());
        assert_eq!(t.next_height(), Height(1));
    }
}
