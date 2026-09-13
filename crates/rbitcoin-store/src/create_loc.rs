//! Packed create locator: 2 B/create (`txout_strides:u8`, `n_out:u8`).
//!
//! Checkpoints in `create.off` (RAM). Loc and overflow stay FdOnly.

use crate::delta_loc::{
    create_table_file, decode_create_pair, load_create_ovf, loc_file_off, loc_window, loc_within,
    open_table_file, pack_create_pair, strides_from_aligned_len, IDX_STRIDE, LOC_WINDOW,
};
use crate::error::StoreError;
use crate::file::{TableFile, FILE_HEADER_LEN};
use rbitcoin_primitives::{Fk, TableKind};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

const SLOT: u64 = 2;
const OFF_SLOT: u64 = 16;
const OVF_SLOT: u64 = 12;

/// One create's txout range, spent range, and true output count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CreateLocPair {
    pub txout: (u64, u64),
    pub spent: (u64, u64),
    pub n_out: u32,
}

/// Per-create append input (8-aligned body starts and txout length).
#[derive(Clone, Copy, Debug)]
pub struct CreateLocAppend {
    pub txout_start: u64,
    pub txout_len: u64,
    pub spent_start: u64,
    pub n_out: u32,
}

pub struct CreateLoc {
    loc: TableFile,
    ovf: TableFile,
    off: TableFile,
    checkpoints: RwLock<Vec<(u64, u64)>>,
    ovf_rows: RwLock<Vec<(u64, u32, u32)>>,
    count: AtomicU64,
}

impl CreateLoc {
    pub fn create(dir: &Path) -> Result<Self, StoreError> {
        Ok(Self {
            loc: create_table_file(&dir.join("create.loc"), TableKind::DeltaLoc)?,
            ovf: create_table_file(&dir.join("create.loc.ovf"), TableKind::DeltaLoc)?,
            off: create_table_file(&dir.join("create.off"), TableKind::ArrayLink)?,
            checkpoints: RwLock::new(Vec::new()),
            ovf_rows: RwLock::new(Vec::new()),
            count: AtomicU64::new(0),
        })
    }

    pub fn open(dir: &Path) -> Result<Self, StoreError> {
        let loc = open_table_file(&dir.join("create.loc"), TableKind::DeltaLoc)?;
        let ovf_path = dir.join("create.loc.ovf");
        let ovf = if ovf_path.exists() {
            open_table_file(&ovf_path, TableKind::DeltaLoc)?
        } else {
            create_table_file(&ovf_path, TableKind::DeltaLoc)?
        };
        let off = open_table_file(&dir.join("create.off"), TableKind::ArrayLink)?;
        let data = loc.data_len();
        if data % SLOT != 0 {
            return Err(StoreError::Corrupt("invariant: create.loc size"));
        }
        let count = data / SLOT;
        let n_win = count / LOC_WINDOW;
        if off.data_len() != n_win * OFF_SLOT {
            return Err(StoreError::Corrupt("invariant: create.off size"));
        }
        let mut checkpoints = vec![(0u64, 0u64); n_win as usize];
        if n_win > 0 {
            let mut bytes = vec![0u8; (n_win * OFF_SLOT) as usize];
            off.read_at(FILE_HEADER_LEN as u64, &mut bytes)?;
            for (i, chunk) in bytes.chunks_exact(OFF_SLOT as usize).enumerate() {
                let txout = u64::from_le_bytes(chunk[0..8].try_into().unwrap());
                let spent = u64::from_le_bytes(chunk[8..16].try_into().unwrap());
                checkpoints[i] = (txout, spent);
            }
        }
        let ovf_rows = load_create_ovf(&ovf)?;
        Ok(Self {
            loc,
            ovf,
            off,
            checkpoints: RwLock::new(checkpoints),
            ovf_rows: RwLock::new(ovf_rows),
            count: AtomicU64::new(count),
        })
    }

    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Acquire)
    }

    pub fn append(&self, recs: &[CreateLocAppend]) -> Result<(), StoreError> {
        if recs.is_empty() {
            return Ok(());
        }
        let base = self.count.load(Ordering::Acquire);
        let mut loc_bytes = Vec::with_capacity(recs.len() * 2);
        let mut ovf_bytes = Vec::new();
        let mut new_ovf: Vec<(u64, u32, u32)> = Vec::new();
        let mut new_offs: Vec<(u64, u64, u64)> = Vec::new();
        for (i, rec) in recs.iter().enumerate() {
            let fk = base + 1 + i as u64;
            if rec.n_out == 0 {
                return Err(StoreError::Corrupt("invariant: create n_out"));
            }
            let strides = strides_from_aligned_len(rec.txout_len)?;
            let spent_len = u64::from(rec.n_out).saturating_mul(IDX_STRIDE);
            if i + 1 < recs.len() {
                if recs[i + 1].txout_start != rec.txout_start.saturating_add(rec.txout_len) {
                    return Err(StoreError::Corrupt("invariant: create.loc txout starts"));
                }
                if recs[i + 1].spent_start != rec.spent_start.saturating_add(spent_len) {
                    return Err(StoreError::Corrupt("invariant: create.loc spent starts"));
                }
            }
            let (s8, n8, ovf) = pack_create_pair(strides, rec.n_out)?;
            loc_bytes.push(s8);
            loc_bytes.push(n8);
            if let Some((os, on)) = ovf {
                let mut row = [0u8; OVF_SLOT as usize];
                row[0..8].copy_from_slice(&fk.to_le_bytes());
                row[8..10].copy_from_slice(&os.to_le_bytes());
                row[10..12].copy_from_slice(&on.to_le_bytes());
                ovf_bytes.extend_from_slice(&row);
                new_ovf.push((fk, u32::from(os), u32::from(on)));
            }
            if fk.is_multiple_of(LOC_WINDOW) {
                new_offs.push((
                    fk / LOC_WINDOW - 1,
                    rec.txout_start.saturating_add(rec.txout_len),
                    rec.spent_start.saturating_add(spent_len),
                ));
            }
        }
        self.loc
            .write_at(loc_file_off(base + 1, SLOT), &loc_bytes)?;
        if !ovf_bytes.is_empty() {
            let ovf_at = FILE_HEADER_LEN as u64 + self.ovf.data_len();
            self.ovf.write_at(ovf_at, &ovf_bytes)?;
            let mut rows = self.ovf_rows.write().unwrap_or_else(|e| e.into_inner());
            if let Some(&(last, _, _)) = rows.last() {
                if new_ovf[0].0 <= last {
                    return Err(StoreError::Corrupt("invariant: create.loc ovf order"));
                }
            }
            rows.extend_from_slice(&new_ovf);
        }
        if !new_offs.is_empty() {
            let mut cps = self.checkpoints.write().unwrap_or_else(|e| e.into_inner());
            let mut blob = Vec::with_capacity(new_offs.len() * OFF_SLOT as usize);
            for &(w, txout_abs, spent_abs) in &new_offs {
                if w as usize != cps.len() {
                    return Err(StoreError::Corrupt("invariant: create.off index"));
                }
                cps.push((txout_abs, spent_abs));
                blob.extend_from_slice(&txout_abs.to_le_bytes());
                blob.extend_from_slice(&spent_abs.to_le_bytes());
            }
            let off_at = FILE_HEADER_LEN as u64 + new_offs[0].0 * OFF_SLOT;
            self.off.write_at(off_at, &blob)?;
        }
        self.count
            .store(base + recs.len() as u64, Ordering::Release);
        Ok(())
    }

    pub fn range_batch(&self, fks: &[Fk]) -> Result<Vec<Option<CreateLocPair>>, StoreError> {
        if fks.is_empty() {
            return Ok(Vec::new());
        }
        let count = self.count.load(Ordering::Acquire);
        let mut out = vec![None; fks.len()];
        let mut jobs: Vec<(usize, u64)> = Vec::new();
        for (i, fk) in fks.iter().enumerate() {
            let Some(id) = fk.get() else { continue };
            if id == 0 || id > count {
                continue;
            }
            jobs.push((i, id));
        }
        if jobs.is_empty() {
            return Ok(out);
        }
        jobs.sort_unstable_by_key(|(_, id)| *id);
        let cps = self.checkpoints.read().unwrap_or_else(|e| e.into_inner());
        let ovf = self.ovf_rows.read().unwrap_or_else(|e| e.into_inner());
        let mut w_i = 0usize;
        while w_i < jobs.len() {
            let w = loc_window(jobs[w_i].1);
            let mut w_j = w_i + 1;
            while w_j < jobs.len() && loc_window(jobs[w_j].1) == w {
                w_j += 1;
            }
            let win_first = w * LOC_WINDOW + 1;
            let win_last = ((w + 1) * LOC_WINDOW).min(count);
            let n = (win_last - win_first + 1) as usize;
            let mut buf = vec![0u8; n * 2];
            self.loc.read_at(loc_file_off(win_first, SLOT), &mut buf)?;
            let (tx0, sp0) = if w == 0 {
                (FILE_HEADER_LEN as u64, FILE_HEADER_LEN as u64)
            } else {
                *cps.get((w - 1) as usize)
                    .ok_or(StoreError::Corrupt("invariant: create.off checkpoint"))?
            };
            let mut tx_ps = vec![0u64; n + 1];
            let mut sp_ps = vec![0u64; n + 1];
            let mut n_outs = vec![0u32; n];
            tx_ps[0] = tx0;
            sp_ps[0] = sp0;
            let mut any_sentinel = false;
            for i in 0..n {
                if buf[i * 2] == 0 || buf[i * 2 + 1] == 0 {
                    any_sentinel = true;
                    break;
                }
            }
            if any_sentinel {
                for i in 0..n {
                    let fk = win_first + i as u64;
                    let (st, n_out) = decode_create_pair(buf[i * 2], buf[i * 2 + 1], fk, &ovf)?;
                    n_outs[i] = n_out;
                    tx_ps[i + 1] =
                        tx_ps[i].saturating_add(u64::from(st).saturating_mul(IDX_STRIDE));
                    sp_ps[i + 1] =
                        sp_ps[i].saturating_add(u64::from(n_out).saturating_mul(IDX_STRIDE));
                }
            } else {
                for i in 0..n {
                    let st = u32::from(buf[i * 2]);
                    let n_out = u32::from(buf[i * 2 + 1]);
                    n_outs[i] = n_out;
                    tx_ps[i + 1] =
                        tx_ps[i].saturating_add(u64::from(st).saturating_mul(IDX_STRIDE));
                    sp_ps[i + 1] =
                        sp_ps[i].saturating_add(u64::from(n_out).saturating_mul(IDX_STRIDE));
                }
            }
            for &(orig, id) in &jobs[w_i..w_j] {
                let within = loc_within(id);
                out[orig] = Some(CreateLocPair {
                    txout: (tx_ps[within], tx_ps[within + 1] - tx_ps[within]),
                    spent: (sp_ps[within], sp_ps[within + 1] - sp_ps[within]),
                    n_out: n_outs[within],
                });
            }
            w_i = w_j;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file::FILE_HEADER_LEN;
    use crate::testutil::TempDir;

    fn rec(txout_start: u64, txout_len: u64, spent_start: u64, n_out: u32) -> CreateLocAppend {
        CreateLocAppend {
            txout_start,
            txout_len,
            spent_start,
            n_out,
        }
    }

    fn chain(n_out: &[u32], txout_len: &[u64]) -> Vec<CreateLocAppend> {
        assert_eq!(n_out.len(), txout_len.len());
        let mut tx = FILE_HEADER_LEN as u64;
        let mut sp = FILE_HEADER_LEN as u64;
        let mut out = Vec::with_capacity(n_out.len());
        for (&n, &tlen) in n_out.iter().zip(txout_len.iter()) {
            out.push(rec(tx, tlen, sp, n));
            tx += tlen;
            sp += u64::from(n) * IDX_STRIDE;
        }
        out
    }

    #[test]
    fn create_loc_n_out_1_and_3() {
        let dir = TempDir::labeled("create-loc-13").unwrap();
        let loc = CreateLoc::create(dir.path()).unwrap();
        loc.append(&chain(&[1, 3], &[16, 32])).unwrap();
        let got = loc.range_batch(&[Fk(1), Fk(2)]).unwrap();
        assert_eq!(
            got[0],
            Some(CreateLocPair {
                txout: (FILE_HEADER_LEN as u64, 16),
                spent: (FILE_HEADER_LEN as u64, 8),
                n_out: 1,
            })
        );
        assert_eq!(
            got[1],
            Some(CreateLocPair {
                txout: (FILE_HEADER_LEN as u64 + 16, 32),
                spent: (FILE_HEADER_LEN as u64 + 8, 24),
                n_out: 3,
            })
        );
        let last = got[1].unwrap();
        assert_eq!(last.txout.0 + last.txout.1, FILE_HEADER_LEN as u64 + 48);
        assert_eq!(last.spent.0 + last.spent.1, FILE_HEADER_LEN as u64 + 32);
    }

    #[test]
    fn create_loc_batch_preserves_caller_order() {
        let dir = TempDir::labeled("create-loc-order").unwrap();
        let loc = CreateLoc::create(dir.path()).unwrap();
        loc.append(&chain(&[1, 1, 1], &[8, 8, 8])).unwrap();
        let got = loc
            .range_batch(&[Fk(3), Fk(1), Fk(2), Fk::NULL, Fk(9)])
            .unwrap();
        assert_eq!(got[0].unwrap().txout.0, FILE_HEADER_LEN as u64 + 16);
        assert_eq!(got[1].unwrap().txout.0, FILE_HEADER_LEN as u64);
        assert_eq!(got[2].unwrap().txout.0, FILE_HEADER_LEN as u64 + 8);
        assert_eq!(got[3], None);
        assert_eq!(got[4], None);
    }

    #[test]
    fn create_loc_window_1024_and_1025() {
        let dir = TempDir::labeled("create-loc-win").unwrap();
        let loc = CreateLoc::create(dir.path()).unwrap();
        let n_out = vec![1u32; 1025];
        let lens = vec![8u64; 1025];
        loc.append(&chain(&n_out, &lens)).unwrap();
        assert_eq!(loc.count(), 1025);
        let got = loc.range_batch(&[Fk(1), Fk(1024), Fk(1025)]).unwrap();
        assert_eq!(got[0].unwrap().txout, (FILE_HEADER_LEN as u64, 8));
        assert_eq!(
            got[1].unwrap().txout,
            (FILE_HEADER_LEN as u64 + 1023 * 8, 8)
        );
        assert_eq!(
            got[2].unwrap().txout,
            (FILE_HEADER_LEN as u64 + 1024 * 8, 8)
        );
        assert_eq!(
            got[2].unwrap().spent,
            (FILE_HEADER_LEN as u64 + 1024 * 8, 8)
        );
        drop(loc);
        let loc = CreateLoc::open(dir.path()).unwrap();
        let got = loc.range_batch(&[Fk(1024), Fk(1025)]).unwrap();
        assert_eq!(
            got[0].unwrap().txout,
            (FILE_HEADER_LEN as u64 + 1023 * 8, 8)
        );
        assert_eq!(
            got[1].unwrap().txout,
            (FILE_HEADER_LEN as u64 + 1024 * 8, 8)
        );
    }

    #[test]
    fn create_loc_n_out_zero_append_corrupt() {
        let dir = TempDir::labeled("create-loc-zero").unwrap();
        let loc = CreateLoc::create(dir.path()).unwrap();
        match loc.append(&[rec(FILE_HEADER_LEN as u64, 8, FILE_HEADER_LEN as u64, 0)]) {
            Err(StoreError::Corrupt(m)) => assert!(m.contains("create n_out"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn create_loc_n_out_256_overflow() {
        let dir = TempDir::labeled("create-loc-256").unwrap();
        let loc = CreateLoc::create(dir.path()).unwrap();
        loc.append(&chain(&[256], &[8])).unwrap();
        let got = loc.range_batch(&[Fk(1)]).unwrap();
        assert_eq!(got[0].unwrap().n_out, 256);
        assert_eq!(got[0].unwrap().spent.1, 256 * 8);
        assert!(dir.path().join("create.loc.ovf").exists());
        drop(loc);
        let loc = CreateLoc::open(dir.path()).unwrap();
        assert_eq!(loc.range_batch(&[Fk(1)]).unwrap()[0].unwrap().n_out, 256);
    }

    #[test]
    fn create_loc_fat_txout_overflow() {
        let dir = TempDir::labeled("create-loc-fat").unwrap();
        let loc = CreateLoc::create(dir.path()).unwrap();
        loc.append(&chain(&[1], &[2048])).unwrap();
        let got = loc.range_batch(&[Fk(1)]).unwrap();
        assert_eq!(got[0].unwrap().txout.1, 2048);
        assert_eq!(got[0].unwrap().n_out, 1);
    }

    #[test]
    fn create_loc_mixed_window_overflow() {
        let dir = TempDir::labeled("create-loc-mix").unwrap();
        let loc = CreateLoc::create(dir.path()).unwrap();
        let mut n_out = vec![1u32; 16];
        n_out[7] = 256;
        let lens = vec![8u64; 16];
        loc.append(&chain(&n_out, &lens)).unwrap();
        let got = loc.range_batch(&[Fk(8), Fk(1), Fk(16)]).unwrap();
        assert_eq!(got[0].unwrap().n_out, 256);
        assert_eq!(got[0].unwrap().spent.1, 256 * 8);
        assert_eq!(got[1].unwrap().n_out, 1);
        assert_eq!(got[2].unwrap().n_out, 1);
        assert_eq!(got[0].unwrap().spent.0, FILE_HEADER_LEN as u64 + 7 * 8);
        assert_eq!(
            got[2].unwrap().spent.0,
            FILE_HEADER_LEN as u64 + 7 * 8 + 256 * 8 + 7 * 8
        );
    }

    #[test]
    fn create_loc_missing_ovf_is_corrupt() {
        let dir = TempDir::labeled("create-loc-missing-ovf").unwrap();
        let loc = CreateLoc::create(dir.path()).unwrap();
        loc.append(&chain(&[256], &[8])).unwrap();
        drop(loc);
        std::fs::remove_file(dir.path().join("create.loc.ovf")).unwrap();
        let loc = CreateLoc::open(dir.path()).unwrap();
        match loc.range_batch(&[Fk(1)]) {
            Err(StoreError::Corrupt(m)) => {
                assert!(m.contains("overflow missing"), "{m}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn spent_len_is_8_times_n_out() {
        assert_eq!(1u64 * IDX_STRIDE, 8);
        assert_eq!(3u64 * IDX_STRIDE, 24);
        assert_eq!(256u64 * IDX_STRIDE, 2048);
    }
}
