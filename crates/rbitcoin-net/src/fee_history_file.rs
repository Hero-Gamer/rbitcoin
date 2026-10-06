//! On-disk copy of the fee history in the mempool dir, so historical fee
//! estimates answer right after a restart.
//!
//! `fee_history` is a snapshot rewritten every [`COMPACT_EVERY`] connects and
//! after each preload; `fee_history.log` is a journal of 52-byte records
//! appended per connect. Both are a cache of `txstat`: a file that is torn,
//! from an older snapshot, in another version, or off the best chain is
//! dropped with a log line and the heights are read from the chain again.
//!
//! ```text
//! fee_history      "RBFH" version:u32 count:u32 n_hashes:u32
//!                  count × (height:u32 txstat_bytes:u32 p10:u64)   p10 MAX = none
//!                  n_hashes × (height:u32 hash:[32])
//!                  check:[8]                                        sha256d prefix
//! fee_history.log  "RBFJ" version:u32 generation:[8]                snapshot check
//!                  n × (height:u32 txstat_bytes:u32 p10:u64 hash:[32] check:[4])
//! ```

use crate::fee_history::HistoricalFeeBlock;
use bitcoin::hashes::{sha256d, Hash};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

const SNAPSHOT_FILE: &str = "fee_history";
const JOURNAL_FILE: &str = "fee_history.log";
const SNAPSHOT_MAGIC: &[u8; 4] = b"RBFH";
const JOURNAL_MAGIC: &[u8; 4] = b"RBFJ";
const VERSION: u32 = 1;
const NO_HURDLE: u64 = u64::MAX;
const SNAPSHOT_HEADER: usize = 16;
const SNAPSHOT_ROW: usize = 16;
const SNAPSHOT_HASH: usize = 36;
const JOURNAL_HEADER: usize = 16;
const JOURNAL_RECORD: usize = 52;

/// Connects between snapshot rewrites.
pub(crate) const COMPACT_EVERY: u32 = 144;

/// Snapshot rows, kept hashes, and journal records in append order.
#[derive(Debug, Default)]
pub(crate) struct LoadedFeeHistory {
    pub(crate) rows: Vec<(u32, HistoricalFeeBlock)>,
    pub(crate) hashes: Vec<(u32, [u8; 32])>,
    pub(crate) journal: Vec<(u32, HistoricalFeeBlock, [u8; 32])>,
    /// Why some or all of the journal was not replayed.
    pub(crate) journal_note: Option<String>,
}

fn check(bytes: &[u8]) -> [u8; 32] {
    sha256d::Hash::hash(bytes).to_byte_array()
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"))
}

fn put_row(out: &mut Vec<u8>, height: u32, block: &HistoricalFeeBlock) {
    out.extend_from_slice(&height.to_le_bytes());
    let bytes = u32::try_from(block.txstat_bytes).unwrap_or(u32::MAX);
    out.extend_from_slice(&bytes.to_le_bytes());
    out.extend_from_slice(&block.p10_sat_kvb.unwrap_or(NO_HURDLE).to_le_bytes());
}

fn row_at(b: &[u8], at: usize) -> (u32, HistoricalFeeBlock) {
    let p10 = u64_at(b, at + 8);
    (
        u32_at(b, at),
        HistoricalFeeBlock {
            p10_sat_kvb: (p10 != NO_HURDLE).then_some(p10),
            txstat_bytes: u64::from(u32_at(b, at + 4)),
        },
    )
}

/// Write the snapshot (temp file, fsync, rename). Returns its generation.
///
/// IO: ~500 KiB and one fsync for 31k mainnet heights, every
/// [`COMPACT_EVERY`] connects on the connect path.
pub(crate) fn write_snapshot(
    dir: &Path,
    rows: &[(u32, HistoricalFeeBlock)],
    hashes: &[(u32, [u8; 32])],
) -> io::Result<[u8; 8]> {
    let mut out = Vec::with_capacity(
        SNAPSHOT_HEADER + rows.len() * SNAPSHOT_ROW + hashes.len() * SNAPSHOT_HASH + 8,
    );
    out.extend_from_slice(SNAPSHOT_MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    out.extend_from_slice(&(hashes.len() as u32).to_le_bytes());
    for (height, block) in rows {
        put_row(&mut out, *height, block);
    }
    for (height, hash) in hashes {
        out.extend_from_slice(&height.to_le_bytes());
        out.extend_from_slice(hash);
    }
    let mut generation = [0u8; 8];
    generation.copy_from_slice(&check(&out)[..8]);
    out.extend_from_slice(&generation);
    let tmp = dir.join(format!("{SNAPSHOT_FILE}.tmp"));
    let mut f = File::create(&tmp)?;
    f.write_all(&out)?;
    f.sync_all()?;
    std::fs::rename(&tmp, dir.join(SNAPSHOT_FILE))?;
    Ok(generation)
}

/// Start an empty journal that extends snapshot `generation`.
pub(crate) fn start_journal(dir: &Path, generation: [u8; 8]) -> io::Result<File> {
    let mut f = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(dir.join(JOURNAL_FILE))?;
    let mut header = Vec::with_capacity(JOURNAL_HEADER);
    header.extend_from_slice(JOURNAL_MAGIC);
    header.extend_from_slice(&VERSION.to_le_bytes());
    header.extend_from_slice(&generation);
    f.write_all(&header)?;
    Ok(f)
}

/// Append one connect. No fsync: a lost tail is refilled from the chain.
pub(crate) fn append(
    journal: &mut File,
    height: u32,
    block: &HistoricalFeeBlock,
    hash: &[u8; 32],
) -> io::Result<()> {
    let mut rec = Vec::with_capacity(JOURNAL_RECORD);
    put_row(&mut rec, height, block);
    rec.extend_from_slice(hash);
    let c = check(&rec);
    rec.extend_from_slice(&c[..4]);
    journal.write_all(&rec)
}

/// Read the snapshot and the journal that extends it. `Ok(None)` when there
/// is no snapshot; `Err` names why an existing snapshot is unusable.
pub(crate) fn load(dir: &Path) -> Result<Option<LoadedFeeHistory>, String> {
    let snap = match std::fs::read(dir.join(SNAPSHOT_FILE)) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("read {SNAPSHOT_FILE}: {e}")),
    };
    if snap.len() < SNAPSHOT_HEADER + 8 || &snap[..4] != SNAPSHOT_MAGIC {
        return Err(format!("{SNAPSHOT_FILE}: not a fee history snapshot"));
    }
    let version = u32_at(&snap, 4);
    if version != VERSION {
        return Err(format!(
            "{SNAPSHOT_FILE}: version {version}, expected {VERSION}"
        ));
    }
    let count = u32_at(&snap, 8) as usize;
    let n_hashes = u32_at(&snap, 12) as usize;
    let body = SNAPSHOT_HEADER + count * SNAPSHOT_ROW + n_hashes * SNAPSHOT_HASH;
    if snap.len() != body + 8 {
        return Err(format!(
            "{SNAPSHOT_FILE}: length {} for {count} rows",
            snap.len()
        ));
    }
    if check(&snap[..body])[..8] != snap[body..] {
        return Err(format!("{SNAPSHOT_FILE}: checksum mismatch"));
    }
    let mut loaded = LoadedFeeHistory::default();
    for i in 0..count {
        loaded
            .rows
            .push(row_at(&snap, SNAPSHOT_HEADER + i * SNAPSHOT_ROW));
    }
    let hashes_at = SNAPSHOT_HEADER + count * SNAPSHOT_ROW;
    for i in 0..n_hashes {
        let at = hashes_at + i * SNAPSHOT_HASH;
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&snap[at + 4..at + 36]);
        loaded.hashes.push((u32_at(&snap, at), hash));
    }

    let journal = match std::fs::read(dir.join(JOURNAL_FILE)) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Some(loaded)),
        Err(e) => {
            loaded.journal_note = Some(format!("read {JOURNAL_FILE}: {e}"));
            return Ok(Some(loaded));
        }
    };
    if journal.len() < JOURNAL_HEADER
        || &journal[..4] != JOURNAL_MAGIC
        || u32_at(&journal, 4) != VERSION
    {
        loaded.journal_note = Some(format!("{JOURNAL_FILE}: bad header"));
        return Ok(Some(loaded));
    }
    if journal[8..16] != snap[body..] {
        loaded.journal_note = Some(format!("{JOURNAL_FILE}: extends an older snapshot"));
        return Ok(Some(loaded));
    }
    for rec in journal[JOURNAL_HEADER..].chunks(JOURNAL_RECORD) {
        if rec.len() < JOURNAL_RECORD || check(&rec[..48])[..4] != rec[48..] {
            loaded.journal_note = Some(format!(
                "{JOURNAL_FILE}: torn after {} records",
                loaded.journal.len()
            ));
            break;
        }
        let (height, block) = row_at(rec, 0);
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&rec[16..48]);
        loaded.journal.push((height, block, hash));
    }
    Ok(Some(loaded))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> std::path::PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("rbitcoin-feehist-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn block(rate: Option<u64>) -> HistoricalFeeBlock {
        HistoricalFeeBlock {
            p10_sat_kvb: rate,
            txstat_bytes: 8,
        }
    }

    fn hash(n: u8) -> [u8; 32] {
        [n; 32]
    }

    #[test]
    fn snapshot_and_journal_round_trip() {
        let d = dir();
        assert!(load(&d).unwrap().is_none(), "no file yet");
        let rows = [(10, block(Some(1_000))), (11, block(None))];
        let generation = write_snapshot(&d, &rows, &[(11, hash(11))]).unwrap();
        let mut j = start_journal(&d, generation).unwrap();
        append(&mut j, 12, &block(Some(2_000)), &hash(12)).unwrap();
        append(&mut j, 12, &block(Some(2_500)), &hash(13)).unwrap();
        drop(j);

        let loaded = load(&d).unwrap().unwrap();
        assert_eq!(loaded.rows, rows);
        assert_eq!(loaded.hashes, [(11, hash(11))]);
        assert_eq!(
            loaded.journal,
            [
                (12, block(Some(2_000)), hash(12)),
                (12, block(Some(2_500)), hash(13))
            ]
        );
        assert_eq!(loaded.journal_note, None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_torn_journal_keeps_the_records_before_the_tear() {
        let d = dir();
        let generation = write_snapshot(&d, &[(1, block(Some(500)))], &[]).unwrap();
        let mut j = start_journal(&d, generation).unwrap();
        append(&mut j, 2, &block(Some(600)), &hash(2)).unwrap();
        append(&mut j, 3, &block(Some(700)), &hash(3)).unwrap();
        j.write_all(&[7u8; 20]).unwrap();
        drop(j);
        let loaded = load(&d).unwrap().unwrap();
        assert_eq!(loaded.journal.len(), 2);
        assert!(loaded.journal_note.unwrap().contains("torn after 2"));

        // a flipped byte inside a record ends the replay there
        let path = d.join(JOURNAL_FILE);
        let mut raw = std::fs::read(&path).unwrap();
        raw[JOURNAL_HEADER + JOURNAL_RECORD + 9] ^= 1;
        std::fs::write(&path, raw).unwrap();
        assert_eq!(load(&d).unwrap().unwrap().journal.len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_journal_from_an_older_snapshot_is_not_replayed() {
        let d = dir();
        let old = write_snapshot(&d, &[(1, block(Some(500)))], &[]).unwrap();
        let mut j = start_journal(&d, old).unwrap();
        append(&mut j, 2, &block(Some(600)), &hash(2)).unwrap();
        drop(j);
        write_snapshot(&d, &[(1, block(Some(500))), (2, block(Some(600)))], &[]).unwrap();
        let loaded = load(&d).unwrap().unwrap();
        assert!(loaded.journal.is_empty());
        assert!(loaded.journal_note.unwrap().contains("older snapshot"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_damaged_or_foreign_snapshot_is_refused() {
        let d = dir();
        write_snapshot(&d, &[(1, block(Some(500)))], &[(1, hash(1))]).unwrap();
        let path = d.join(SNAPSHOT_FILE);
        let good = std::fs::read(&path).unwrap();

        let mut flipped = good.clone();
        flipped[SNAPSHOT_HEADER + 9] ^= 1;
        std::fs::write(&path, &flipped).unwrap();
        assert!(load(&d).unwrap_err().contains("checksum"));

        let mut v2 = good.clone();
        v2[4..8].copy_from_slice(&2u32.to_le_bytes());
        std::fs::write(&path, &v2).unwrap();
        assert!(load(&d).unwrap_err().contains("version 2"));

        std::fs::write(&path, &good[..good.len() - 1]).unwrap();
        assert!(load(&d).unwrap_err().contains("length"));

        std::fs::write(&path, b"not a snapshot at all, but long enough").unwrap();
        assert!(load(&d).unwrap_err().contains("not a fee history"));

        std::fs::write(&path, &good[..16]).unwrap();
        assert!(
            load(&d).unwrap_err().contains("not a fee history"),
            "a header without the checksum trailer is not a snapshot"
        );
        write_snapshot(&d, &[], &[]).unwrap();
        let empty = load(&d).unwrap().unwrap();
        assert!(empty.rows.is_empty() && empty.hashes.is_empty());

        let generation = write_snapshot(&d, &[(1, block(Some(500)))], &[]).unwrap();
        drop(start_journal(&d, generation).unwrap());
        let exact = load(&d).unwrap().unwrap();
        assert!(exact.journal.is_empty());
        assert_eq!(exact.journal_note, None);
        let mut j = start_journal(&d, generation).unwrap();
        append(&mut j, 2, &block(Some(600)), &hash(2)).unwrap();
        drop(j);
        let jpath = d.join(JOURNAL_FILE);
        let mut raw = std::fs::read(&jpath).unwrap();
        raw[0] = b'X';
        std::fs::write(&jpath, &raw).unwrap();
        let foreign = load(&d).unwrap().unwrap();
        assert!(foreign.journal.is_empty(), "{foreign:?}");
        assert!(
            foreign.journal_note.unwrap().contains("bad header"),
            "a long journal with a bad magic is not replayed"
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}
