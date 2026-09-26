//! BIP158 basic filter index (`blockfilter.idx/` + `blockfilter.body/`).
//!
//! Additive side table: Class A is unchanged and a missing dir is "not built".
//! Slots are dense from height 0 on the best chain. A reorg truncates above
//! the new tip and those heights are written again.
//!
//! One fixed idx slot per height carries the record's offset and length, so
//! any height's hash or header is one idx pread, and any run of filters is one
//! idx pread plus one body pread per segment. The slot keeps `header_fk` so
//! open can drop slots a crash left on a stale branch.
//!
//! ```text
//! blockfilter.idx/meta      fmt:u32=1
//! blockfilter.idx/NNNNNN    slot[i] = off:u32 ‖ len:u32 ‖ header_fk:u64
//!                                     ‖ filter_hash:[u8;32] ‖ filter_header:[u8;32]
//! blockfilter.body/NNNNNN   filter bytes (Core `BlockFilter` content), records back to back
//! ```
//!
//! When the next record's **start** would exceed `u32::MAX`, a new `NNNNNN`
//! pair starts (the sp_tweaks rule).
//!
//! **Commit order:** body pwrite → body `sync_data` → idx pwrite → idx
//! `sync_data`, all past the visible end, then the slot count is published.
//! Readers never wait on commit IO. After power loss the idx HWM may cover a
//! torn slot and the body may hold bytes of an uncommitted record: open keeps
//! the longest prefix of slots whose records run back to back inside the body,
//! cuts the body to its end, and the caller checks `header_fk` against
//! `confirmed[]`.

use crate::error::StoreError;
use crate::file::{TableFile, FILE_HEADER_LEN};
use rbitcoin_primitives::{Fk, Height, TableKind};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};

const SLOT: u64 = 80;
const IDX_FMT: u32 = 1;
const HDR: u64 = FILE_HEADER_LEN as u64;

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
    /// End of the last committed record (`HDR` when empty).
    body_end: u64,
    idx: TableFile,
    body: TableFile,
}

struct Inner {
    segs: Vec<Seg>,
}

impl Inner {
    fn next(&self) -> u64 {
        self.segs.iter().map(|s| s.n_slots).sum()
    }

    fn locate(&self, height: Height) -> Option<(usize, u64)> {
        let g = u64::from(height.0);
        self.segs
            .iter()
            .enumerate()
            .find(|(_, s)| g >= s.first_slot && g < s.first_slot + s.n_slots)
            .map(|(si, s)| (si, g - s.first_slot))
    }
}

/// Height-dense basic filter table.
pub struct BlockFilterTable {
    dir: PathBuf,
    inner: RwLock<Inner>,
    /// Serializes `put` and `truncate_through` (one writer at a time).
    writer: Mutex<()>,
}

/// A filter's bytes and idx facts.
pub type StoredBlockFilter = (Vec<u8>, BlockFilterSlot);
/// Idx rows grouped by segment index.
type SegRows = Vec<(usize, Vec<SlotRow>)>;

struct SlotRow {
    off: u32,
    len: u32,
    slot: BlockFilterSlot,
}

fn encode_slot(off: u32, len: u32, s: &BlockFilterSlot) -> [u8; SLOT as usize] {
    let mut b = [0u8; SLOT as usize];
    b[0..4].copy_from_slice(&off.to_le_bytes());
    b[4..8].copy_from_slice(&len.to_le_bytes());
    b[8..16].copy_from_slice(&s.header_fk.0.to_le_bytes());
    b[16..48].copy_from_slice(&s.filter_hash);
    b[48..80].copy_from_slice(&s.filter_header);
    b
}

fn decode_slot(b: &[u8]) -> SlotRow {
    let mut filter_hash = [0u8; 32];
    filter_hash.copy_from_slice(&b[16..48]);
    let mut filter_header = [0u8; 32];
    filter_header.copy_from_slice(&b[48..80]);
    SlotRow {
        off: u32::from_le_bytes(b[0..4].try_into().unwrap()),
        len: u32::from_le_bytes(b[4..8].try_into().unwrap()),
        slot: BlockFilterSlot {
            header_fk: Fk(u64::from_le_bytes(b[8..16].try_into().unwrap())),
            filter_hash,
            filter_header,
        },
    }
}

impl SlotRow {
    fn end(&self) -> u64 {
        u64::from(self.off) + u64::from(self.len)
    }
}

impl Seg {
    fn read_rows(&self, local: u64, n: u64) -> Result<Vec<SlotRow>, StoreError> {
        let mut b = vec![0u8; (n * SLOT) as usize];
        self.idx.read_at(HDR + local * SLOT, &mut b)?;
        Ok(b.chunks_exact(SLOT as usize).map(decode_slot).collect())
    }
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
            body_end: HDR,
            idx,
            body,
        })
    }

    fn with_segs(dir: &Path, segs: Vec<Seg>) -> Self {
        Self {
            dir: dir.to_path_buf(),
            inner: RwLock::new(Inner { segs }),
            writer: Mutex::new(()),
        }
    }

    fn create(dir: &Path) -> Result<Self, StoreError> {
        std::fs::create_dir_all(Self::idx_dir(dir)).map_err(|e| StoreError::io(dir, e))?;
        std::fs::create_dir_all(Self::body_dir(dir)).map_err(|e| StoreError::io(dir, e))?;
        let meta = TableFile::create(Self::meta_path(dir), TableKind::ArrayLink)?;
        meta.write_at_pwrite(HDR, &IDX_FMT.to_le_bytes())?;
        meta.flush()?;
        Ok(Self::with_segs(dir, vec![Self::create_seg(dir, 0, 0)?]))
    }

    fn open(dir: &Path) -> Result<Self, StoreError> {
        let meta = TableFile::open(Self::meta_path(dir), TableKind::ArrayLink)?;
        if meta.logical_len() < HDR + 4 {
            return Err(StoreError::Corrupt("blockfilter.idx/meta short"));
        }
        let mut fmt = [0u8; 4];
        meta.read_at(HDR, &mut fmt)?;
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
            let mut seg = Seg {
                file_id,
                first_slot,
                n_slots: idx.logical_len().saturating_sub(HDR) / SLOT,
                body_end: HDR,
                idx,
                body,
            };
            Self::clamp_torn_tail(&mut seg)?;
            first_slot += seg.n_slots;
            segs.push(seg);
        }
        if segs.is_empty() {
            return Err(StoreError::Corrupt("blockfilter no segments"));
        }
        Ok(Self::with_segs(dir, segs))
    }

    /// Drop trailing slots until the last one chains onto its predecessor and
    /// ends inside the body, then cut the idx and body to it.
    ///
    /// Commits only append, so a crash can only tear the tail: this reads a
    /// few slots at the end, not the whole idx.
    fn clamp_torn_tail(seg: &mut Seg) -> Result<(), StoreError> {
        let body_len = seg.body.logical_len();
        let mut keep = seg.n_slots;
        let end = loop {
            let Some(last) = keep.checked_sub(1) else {
                break HDR;
            };
            let first = last.saturating_sub(1);
            let rows = seg.read_rows(first, last - first + 1)?;
            let row = &rows[rows.len() - 1];
            let start = if last == 0 { HDR } else { rows[0].end() };
            if u64::from(row.off) == start && row.end() <= body_len {
                break row.end();
            }
            keep = last;
        };
        let idx_len = HDR + keep * SLOT;
        if keep < seg.n_slots || seg.idx.logical_len() != idx_len || body_len != end {
            rbitcoin_log::warn!(
                "blockfilter: dropping torn tail file={:06} slots {}→{} body {}→{}",
                seg.file_id,
                seg.n_slots,
                keep,
                body_len,
                end
            );
            seg.idx.set_logical_len(idx_len)?;
            seg.idx.flush()?;
            seg.body.set_logical_len(end)?;
            seg.body.flush()?;
        }
        seg.n_slots = keep;
        seg.body_end = end;
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

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Inner> {
        self.inner.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Inner> {
        self.inner.write().unwrap_or_else(|e| e.into_inner())
    }

    /// Next height this table accepts (0 when empty).
    pub fn next_height(&self) -> Height {
        Height(self.read().next() as u32)
    }

    /// Idx facts at `height` (one pread). `None` past the watermark.
    pub fn slot(&self, height: Height) -> Result<Option<BlockFilterSlot>, StoreError> {
        Ok(self.slots(height, height)?.and_then(|mut v| v.pop()))
    }

    /// Idx rows for `start..=end` grouped by segment (one pread each).
    fn rows(inner: &Inner, start: Height, end: Height) -> Result<Option<SegRows>, StoreError> {
        if start > end || inner.locate(end).is_none() {
            return Ok(None);
        }
        let Some((mut si, mut local)) = inner.locate(start) else {
            return Ok(None);
        };
        let mut left = u64::from(end.0 - start.0) + 1;
        let mut out = Vec::new();
        while left > 0 {
            let seg = &inner.segs[si];
            let run = left.min(seg.n_slots - local);
            out.push((si, seg.read_rows(local, run)?));
            left -= run;
            si += 1;
            local = 0;
        }
        Ok(Some(out))
    }

    /// Idx facts for `start..=end`. `None` when `end` is past the watermark.
    pub fn slots(
        &self,
        start: Height,
        end: Height,
    ) -> Result<Option<Vec<BlockFilterSlot>>, StoreError> {
        let inner = self.read();
        Ok(Self::rows(&inner, start, end)?.map(|groups| {
            groups
                .into_iter()
                .flat_map(|(_, r)| r)
                .map(|r| r.slot)
                .collect()
        }))
    }

    /// Filters and idx facts for `start..=end`: one idx pread and one body
    /// pread per segment. `None` when `end` is past the watermark.
    pub fn filters(
        &self,
        start: Height,
        end: Height,
    ) -> Result<Option<Vec<StoredBlockFilter>>, StoreError> {
        let inner = self.read();
        let Some(groups) = Self::rows(&inner, start, end)? else {
            return Ok(None);
        };
        let mut out = Vec::with_capacity((end.0 - start.0 + 1) as usize);
        for (si, rows) in groups {
            let lo = u64::from(rows[0].off);
            let hi = rows[rows.len() - 1].end();
            let mut span = vec![0u8; (hi - lo) as usize];
            inner.segs[si].body.read_at(lo, &mut span)?;
            for r in rows {
                let at = (u64::from(r.off) - lo) as usize;
                let bytes = span
                    .get(at..at + r.len as usize)
                    .ok_or(StoreError::Corrupt(
                        "invariant: blockfilter slot outside span",
                    ))?;
                out.push((bytes.to_vec(), r.slot));
            }
        }
        Ok(Some(out))
    }

    /// Filter bytes and idx facts at `height`. `None` past the watermark.
    pub fn filter(&self, height: Height) -> Result<Option<StoredBlockFilter>, StoreError> {
        Ok(self.filters(height, height)?.and_then(|mut v| v.pop()))
    }

    /// Append consecutive heights starting at [`Self::next_height`], durably.
    pub fn put(&self, items: &[BlockFilterRecord<'_>]) -> Result<(), StoreError> {
        let _writer = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        let next = self.read().next();
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
            if self.read().segs.last().map_or(0, |s| s.body_end) > u64::from(u32::MAX) {
                let mut inner = self.write();
                let file_id = u32::try_from(inner.segs.len())
                    .map_err(|_| StoreError::Corrupt("blockfilter too many segments"))?;
                let seg = Self::create_seg(&self.dir, file_id, inner.next())?;
                inner.segs.push(seg);
            }
            let (n, end) = {
                let inner = self.read();
                let tail = inner
                    .segs
                    .last()
                    .ok_or(StoreError::Corrupt("blockfilter no segments"))?;
                let mut body = Vec::new();
                let mut idx_blob = Vec::new();
                let mut at = tail.body_end;
                while i < items.len() {
                    let (Ok(off), Ok(len)) =
                        (u32::try_from(at), u32::try_from(items[i].filter.len()))
                    else {
                        break;
                    };
                    idx_blob.extend_from_slice(&encode_slot(off, len, &items[i].slot));
                    body.extend_from_slice(items[i].filter);
                    at += u64::from(len);
                    i += 1;
                }
                if idx_blob.is_empty() {
                    return Err(StoreError::Corrupt("blockfilter body exceeds u32 off"));
                }
                tail.body.write_at_pwrite(tail.body_end, &body)?;
                tail.body.flush()?;
                tail.idx
                    .write_at_pwrite(HDR + tail.n_slots * SLOT, &idx_blob)?;
                tail.idx.flush()?;
                (idx_blob.len() as u64 / SLOT, at)
            };
            let mut inner = self.write();
            let tail = inner
                .segs
                .last_mut()
                .ok_or(StoreError::Corrupt("blockfilter no segments"))?;
            tail.n_slots += n;
            tail.body_end = end;
        }
        Ok(())
    }

    /// Drop heights above `tip` (`None` drops every slot).
    pub fn truncate_through(&self, tip: Option<Height>) -> Result<(), StoreError> {
        let _writer = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        let keep = tip.map_or(0, |h| u64::from(h.0) + 1);
        let mut inner = self.write();
        if keep >= inner.next() {
            return Ok(());
        }
        let si = match keep.checked_sub(1) {
            None => 0,
            Some(last) => inner
                .segs
                .iter()
                .position(|s| last >= s.first_slot && last < s.first_slot + s.n_slots)
                .ok_or(StoreError::Corrupt("blockfilter truncate locate"))?,
        };
        let seg = &mut inner.segs[si];
        let local_keep = keep - seg.first_slot;
        if local_keep < seg.n_slots {
            let body_end = u64::from(seg.read_rows(local_keep, 1)?[0].off);
            seg.idx.set_logical_len(HDR + local_keep * SLOT)?;
            seg.idx.flush()?;
            seg.body.set_logical_len(body_end)?;
            seg.body.flush()?;
            seg.n_slots = local_keep;
            seg.body_end = body_end;
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
        assert_eq!(
            t.slots(Height(3), Height(9)).unwrap().unwrap(),
            (3..=9).map(slot).collect::<Vec<_>>()
        );
        assert!(t.slots(Height(3), Height(10)).unwrap().is_none());
        let run = t.filters(Height(6), Height(8)).unwrap().unwrap();
        assert_eq!(
            run[1],
            (vec![7u8; 10], slot(7)),
            "range read matches single"
        );
        assert!(t.slot(Height(10)).unwrap().is_none());
        let gap = t.put(&[BlockFilterRecord {
            height: Height(11),
            slot: slot(11),
            filter: &[1],
        }]);
        assert!(matches!(gap, Err(StoreError::Corrupt(m)) if m.contains("next height")));
    }

    /// Crash states a live session cannot produce on demand: a commit that
    /// synced body bytes but not its slots, then an idx HWM covering a slot
    /// whose record is cut short. Open keeps the committed prefix each time.
    #[test]
    fn open_keeps_the_committed_prefix_after_torn_commits() {
        let dir = tmp_dir();
        let body = BlockFilterTable::seg_body_path(&dir, 0);
        let end_of = |h: u64| FILE_HEADER_LEN as u64 + (0..=h).map(|i| 3 + i).sum::<u64>();
        let t = BlockFilterTable::open_or_create(&dir).unwrap();
        put_range(&t, 0, 4);
        drop(t);

        set_file_hwm(&body, end_of(4) + 50);
        {
            let mut f = fs::OpenOptions::new().write(true).open(&body).unwrap();
            f.seek(SeekFrom::Start(end_of(4))).unwrap();
            f.write_all(&[0xab; 50]).unwrap();
        }
        let t = BlockFilterTable::open_or_create(&dir).unwrap();
        assert_eq!(t.next_height(), Height(5));
        assert_eq!(t.filter(Height(4)).unwrap().unwrap().0, vec![4u8; 7]);
        put_range(&t, 5, 5);
        assert_eq!(t.filter(Height(5)).unwrap().unwrap().0, vec![5u8; 8]);
        drop(t);

        set_file_hwm(&body, end_of(2) + 1);
        let t = BlockFilterTable::open_or_create(&dir).unwrap();
        assert_eq!(t.next_height(), Height(3), "record 3 is torn");
        put_range(&t, 3, 4);
        assert_eq!(t.filter(Height(4)).unwrap().unwrap().0, vec![4u8; 7]);
    }

    #[test]
    fn rolls_a_segment_when_the_next_start_passes_u32() {
        let dir = tmp_dir();
        let t = BlockFilterTable::open_or_create(&dir).unwrap();
        put_range(&t, 0, 0);
        drop(t);
        // Record 0 now ends one past u32::MAX (sparse body, slot len widened).
        let big = u32::MAX - FILE_HEADER_LEN as u32 + 1;
        let idx = BlockFilterTable::seg_idx_path(&dir, 0);
        let mut raw = fs::read(&idx).unwrap();
        raw[FILE_HEADER_LEN + 4..FILE_HEADER_LEN + 8].copy_from_slice(&big.to_le_bytes());
        fs::write(&idx, &raw).unwrap();
        set_file_hwm(
            &BlockFilterTable::seg_body_path(&dir, 0),
            u64::from(u32::MAX) + 1,
        );

        let t = BlockFilterTable::open_or_create(&dir).unwrap();
        assert_eq!(t.next_height(), Height(1));
        put_range(&t, 1, 2);
        assert!(BlockFilterTable::seg_body_path(&dir, 1).is_file());
        assert_eq!(
            t.filters(Height(1), Height(2)).unwrap().unwrap(),
            vec![(vec![1u8; 4], slot(1)), (vec![2u8; 5], slot(2))]
        );
        assert_eq!(
            t.slots(Height(0), Height(2)).unwrap().unwrap(),
            (0..=2).map(slot).collect::<Vec<_>>(),
            "range read crosses the segment roll"
        );
        t.truncate_through(Some(Height(0))).unwrap();
        assert!(!BlockFilterTable::seg_body_path(&dir, 1).exists());
        assert_eq!(t.next_height(), Height(1));
    }
}
