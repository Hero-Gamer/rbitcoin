//! Growable dense array of little-endian `u64` values.
//!
//! 0-based indexing. Used for Class C: `confirmed[height]`, `header_txs` arrays, etc.
//!
//! # L2 write-behind (phase 6)
//!
//! Compact **array** tables (`confirmed`, `header_txs_*`; body ≤
//! [`class_c_inram_max_bytes`]) load fully into process RAM on open.
//! `strong_tx` is always L2 (see [`crate::chain::StrongTxTable`]).
//! Mutates update the `Vec` only and record the lowest unstored index. Disk
//! is updated on `Self::flush` / barrier by reading those live slots:
//!
//! - **Append** (unstored index ≥ last flushed len): pwrite the new suffix.
//! - **Truncate**: shrink the published length only.
//! - **In-prefix mutate**: pwrite from the first unstored index through the
//!   length recorded when the flush started, then publish that length even
//!   when it equals the previous disk length.
//!
//! Each pwrite is at most [`FLUSH_SCRATCH`] bytes. The unstored mark clears
//! only after that persist succeeds. A store that starts after the flush has
//! recorded its range is refused until the flush returns.
//!
//! Tables over the cap stay pure fd L0 (per-slot write-through). The flush
//! does not read that path.

use crate::error::StoreError;
use crate::file::{TableFile, FILE_HEADER_LEN};
use rbitcoin_primitives::TableKind;
use std::marker::PhantomData;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, RwLock};

/// Capability for the one Class C writer.
///
/// `!Send` and `!Sync`: a shared [`crate::Store`] cannot carry this onto
/// another thread. Confirm-write, tip-accept, and startup each mint one on
/// the thread that stores. The flush thread reads live slots and does not
/// take this token.
pub struct ClassCWrite {
    _thread: PhantomData<*const ()>,
}

impl Default for ClassCWrite {
    fn default() -> Self {
        Self::new()
    }
}

impl ClassCWrite {
    /// Mint a writer capability on the calling thread.
    pub fn new() -> Self {
        Self {
            _thread: PhantomData,
        }
    }

    /// Same as [`Self::new`]. Tests are not the confirm-write thread.
    #[doc(hidden)]
    pub fn for_test() -> Self {
        Self::new()
    }
}

#[doc(hidden)]
pub const ELEM: u64 = 8;

/// Bytes of the fixed pwrite buffer. A flush does not allocate a second image.
#[doc(hidden)]
pub const FLUSH_SCRATCH: usize = 4096;

const CLEAN: u64 = u64::MAX;

/// Where a flush calls its test hook: after the range is recorded, before the
/// write; or after the persist, before the unstored mark clears.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[doc(hidden)]
pub enum FlushStep {
    Write,
    Clean,
}

/// Default max body size for L2 InRam (256 MiB). Over → pure FdOnly L0.
pub const DEFAULT_CLASS_C_INRAM_MAX_BYTES: u64 = 256 * 1024 * 1024;

/// Cap for loading Class C arrays into process RAM.
///
/// Env: `RBITCOIN_CLASS_C_INRAM_MAX_MB` (integer MiB). Unset → 256 MiB.
pub fn class_c_inram_max_bytes() -> u64 {
    match std::env::var("RBITCOIN_CLASS_C_INRAM_MAX_MB") {
        Ok(s) => {
            let mb: u64 = s.trim().parse().unwrap_or(256);
            mb.saturating_mul(1024 * 1024)
        }
        Err(_) => DEFAULT_CLASS_C_INRAM_MAX_BYTES,
    }
}

/// Pwrite `len` bytes at `base` in [`FLUSH_SCRATCH`] chunks. `fill` loads one
/// chunk from the live image. Returns the largest chunk written.
pub(crate) fn pwrite_scratched(
    mut write: impl FnMut(u64, &[u8]) -> Result<(), StoreError>,
    base: u64,
    len: u64,
    mut fill: impl FnMut(&mut [u8], u64) -> Result<(), StoreError>,
) -> Result<usize, StoreError> {
    let mut scratch = [0u8; FLUSH_SCRATCH];
    let mut off = 0u64;
    let mut max_chunk = 0usize;
    while off < len {
        let n = ((len - off) as usize).min(FLUSH_SCRATCH);
        fill(&mut scratch[..n], off)?;
        max_chunk = max_chunk.max(n);
        write(base + off, &scratch[..n])?;
        off += n as u64;
    }
    Ok(max_chunk)
}

pub struct ArrayTable {
    pub file: TableFile,
    pub len: AtomicU64,
    /// `Some` = L2 authoritative image; `None` = L0 fd-only path.
    pub data: RwLock<Option<Vec<u64>>>,
    /// Lowest index not yet persisted. [`CLEAN`] means none.
    pub unstored_from: AtomicU64,
    /// True from the moment a flush records its range until it returns.
    pub flushing: AtomicBool,
    /// Element count last successfully flushed to disk (L2).
    pub disk_len: AtomicU64,
    /// One flusher at a time.
    pub flush_lock: Mutex<()>,
    /// Largest chunk the last flush passed to `write_at`.
    #[doc(hidden)]
    pub flush_chunk_max: AtomicUsize,
}

/// Clears `flushing` on drop. On success, also publishes `disk_len` and
/// clears the unstored mark, in that order, while stores are still refused.
#[must_use]
struct EndFlush<'a> {
    table: &'a ArrayTable,
    persisted: Option<u64>,
}

impl Drop for EndFlush<'_> {
    fn drop(&mut self) {
        if let Some(n) = self.persisted {
            self.table.disk_len.store(n, Ordering::Release);
            self.table.unstored_from.store(CLEAN, Ordering::Release);
        }
        self.table.flushing.store(false, Ordering::Release);
    }
}

impl ArrayTable {
    pub fn create(path: impl AsRef<Path>, kind: TableKind) -> Result<Self, StoreError> {
        let file = TableFile::create(path.as_ref(), kind)?;
        Ok(Self::new(file, Some(Vec::new()), 0))
    }

    pub fn open(path: impl AsRef<Path>, kind: TableKind) -> Result<Self, StoreError> {
        let file = TableFile::open(path.as_ref(), kind)?;
        let body = file.logical_len().saturating_sub(FILE_HEADER_LEN as u64);
        if body % ELEM != 0 {
            return Err(StoreError::Corrupt("array table size"));
        }
        let n = body / ELEM;
        let data = if body <= class_c_inram_max_bytes() {
            let mut v = vec![0u64; n as usize];
            if n > 0 {
                let mut bytes = vec![0u8; body as usize];
                file.read_at(FILE_HEADER_LEN as u64, &mut bytes)?;
                for (i, chunk) in bytes.chunks_exact(8).enumerate() {
                    v[i] = u64::from_le_bytes(chunk.try_into().unwrap());
                }
            }
            Some(v)
        } else {
            None
        };
        Ok(Self::new(file, data, n))
    }

    fn new(file: TableFile, data: Option<Vec<u64>>, n: u64) -> Self {
        Self {
            file,
            len: AtomicU64::new(n),
            data: RwLock::new(data),
            unstored_from: AtomicU64::new(CLEAN),
            flushing: AtomicBool::new(false),
            disk_len: AtomicU64::new(n),
            flush_lock: Mutex::new(()),
            flush_chunk_max: AtomicUsize::new(0),
        }
    }

    pub fn len(&self) -> u64 {
        self.len.load(Ordering::Acquire)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[doc(hidden)]
    pub fn offset(index: u64) -> u64 {
        FILE_HEADER_LEN as u64 + index * ELEM
    }

    fn refuse_while_flushing(&self) -> Result<(), StoreError> {
        if self.flushing.load(Ordering::Acquire) {
            Err(StoreError::Rejected("class C store during flush"))
        } else {
            Ok(())
        }
    }

    fn mark_unstored(&self, index: u64) {
        let _ = self.unstored_from.fetch_min(index, Ordering::AcqRel);
    }

    pub fn get(&self, index: u64) -> Result<u64, StoreError> {
        let len = self.len.load(Ordering::Acquire);
        if index >= len {
            return Ok(0);
        }
        let guard = self.data.read().unwrap_or_else(|e| e.into_inner());
        if let Some(ref v) = *guard {
            // `len` was read before the lock; a truncate may have landed since.
            return Ok(v.get(index as usize).copied().unwrap_or(0));
        }
        drop(guard);
        let mut buf = [0u8; 8];
        self.file.read_at(Self::offset(index), &mut buf)?;
        Ok(u64::from_le_bytes(buf))
    }

    pub fn set(&self, _write: &ClassCWrite, index: u64, value: u64) -> Result<(), StoreError> {
        let mut guard = self.data.write().unwrap_or_else(|e| e.into_inner());
        self.refuse_while_flushing()?;
        if let Some(ref mut v) = *guard {
            let need = index as usize + 1;
            if v.len() < need {
                v.resize(need, 0);
            }
            v[index as usize] = value;
            self.len.store(v.len() as u64, Ordering::Release);
            self.mark_unstored(index);
            return Ok(());
        }
        drop(guard);
        let len = self.len.load(Ordering::Acquire);
        if index >= len {
            for i in len..index {
                self.file.write_at(Self::offset(i), &0u64.to_le_bytes())?;
            }
            self.file
                .write_at(Self::offset(index), &value.to_le_bytes())?;
            self.len.store(index + 1, Ordering::Release);
        } else {
            self.file
                .write_at(Self::offset(index), &value.to_le_bytes())?;
        }
        Ok(())
    }

    pub fn set_many(&self, _write: &ClassCWrite, pairs: &[(u64, u64)]) -> Result<(), StoreError> {
        if pairs.is_empty() {
            return Ok(());
        }
        let max_idx = pairs.iter().map(|(i, _)| *i).max().unwrap();
        let min_idx = pairs.iter().map(|(i, _)| *i).min().unwrap();
        let mut guard = self.data.write().unwrap_or_else(|e| e.into_inner());
        self.refuse_while_flushing()?;
        if let Some(ref mut v) = *guard {
            let need = max_idx as usize + 1;
            if v.len() < need {
                v.resize(need, 0);
            }
            for &(index, value) in pairs {
                v[index as usize] = value;
            }
            self.len.store(v.len() as u64, Ordering::Release);
            self.mark_unstored(min_idx);
            return Ok(());
        }
        drop(guard);
        let len = self.len.load(Ordering::Acquire);
        if max_idx >= len {
            let new_len = max_idx + 1;
            let gap = new_len - len;
            const CHUNK: usize = 1024 * 1024;
            let mut offset = Self::offset(len);
            let mut remaining = gap * ELEM;
            let zeros = vec![0u8; CHUNK];
            while remaining > 0 {
                let n = remaining.min(zeros.len() as u64) as usize;
                self.file.write_at(offset, &zeros[..n])?;
                offset += n as u64;
                remaining -= n as u64;
            }
            self.len.store(new_len, Ordering::Release);
        }
        for &(index, value) in pairs {
            self.file
                .write_at(Self::offset(index), &value.to_le_bytes())?;
        }
        Ok(())
    }

    pub fn truncate(&self, _write: &ClassCWrite, new_len: u64) -> Result<(), StoreError> {
        let len = self.len.load(Ordering::Acquire);
        if new_len > len {
            return Err(StoreError::Corrupt("array truncate grows"));
        }
        let mut guard = self.data.write().unwrap_or_else(|e| e.into_inner());
        self.refuse_while_flushing()?;
        if let Some(ref mut v) = *guard {
            v.truncate(new_len as usize);
            self.len.store(new_len, Ordering::Release);
            // Length-only publish. Mark the start so flush takes the shrink path.
            self.mark_unstored(0);
            return Ok(());
        }
        drop(guard);
        self.len.store(new_len, Ordering::Release);
        let logical = FILE_HEADER_LEN as u64 + new_len * ELEM;
        self.file.set_logical_len(logical)?;
        Ok(())
    }

    /// In-RAM L2 image bytes (0 when FdOnly).
    pub fn l2_resident_bytes(&self) -> u64 {
        let g = self.data.read().unwrap_or_else(|e| e.into_inner());
        g.as_ref()
            .map(|v| (v.len() as u64).saturating_mul(ELEM))
            .unwrap_or(0)
    }

    pub fn flush_dirty(&self) -> Result<(), StoreError> {
        self.flush_dirty_between(|_, _| {})
    }

    #[doc(hidden)]
    pub fn flush_dirty_between(
        &self,
        mut between: impl FnMut(&Self, FlushStep),
    ) -> Result<(), StoreError> {
        let _one = self.flush_lock.lock().unwrap_or_else(|e| e.into_inner());
        let (from, n, disk) = {
            let guard = self.data.read().unwrap_or_else(|e| e.into_inner());
            let Some(ref v) = *guard else {
                return Ok(());
            };
            let from = self.unstored_from.load(Ordering::Acquire);
            if from == CLEAN {
                return Ok(());
            }
            self.flushing.store(true, Ordering::Release);
            (from, v.len() as u64, self.disk_len.load(Ordering::Acquire))
        };
        let mut end = EndFlush {
            table: self,
            persisted: None,
        };
        between(self, FlushStep::Write);
        // Disconnect shrinks `confirmed` once per block. Rewriting the kept
        // prefix would copy the whole image on that path.
        if n >= disk {
            let start = from.min(disk);
            if start < n {
                let max = pwrite_scratched(
                    |at, chunk| self.file.write_at(at, chunk),
                    Self::offset(start),
                    (n - start) * ELEM,
                    |dst, off| {
                        let guard = self.data.read().unwrap_or_else(|e| e.into_inner());
                        let v = guard
                            .as_ref()
                            .ok_or(StoreError::Corrupt("class C flush lost its image"))?;
                        let base = start as usize + (off as usize) / 8;
                        for (i, slot) in dst.chunks_exact_mut(8).enumerate() {
                            slot.copy_from_slice(&v[base + i].to_le_bytes());
                        }
                        Ok(())
                    },
                )?;
                self.flush_chunk_max.store(max, Ordering::Relaxed);
            }
        }
        let logical = FILE_HEADER_LEN as u64 + n * ELEM;
        self.file.set_logical_len(logical)?;
        between(self, FlushStep::Clean);
        end.persisted = Some(n);
        Ok(())
    }

    pub fn flush(&self) -> Result<(), StoreError> {
        self.flush_dirty()?;
        self.file.flush()
    }

    pub fn flush_async(&self) -> Result<(), StoreError> {
        self.flush_dirty()?;
        self.file.flush_async()
    }
}
