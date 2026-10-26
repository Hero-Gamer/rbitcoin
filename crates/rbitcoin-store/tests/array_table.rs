use rbitcoin_primitives::TableKind;
use rbitcoin_store::array_table::*;
use rbitcoin_store::error::StoreError;
use rbitcoin_store::file::FILE_HEADER_LEN;
use std::sync::atomic::Ordering;

fn tmp_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rbitcoin-array-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

/// `get` checks `len` before taking the read lock, so a `truncate` can
/// land in between. The reader must see the shorter table, not panic.
#[test]
fn get_racing_truncate_does_not_panic() {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    let path = tmp_path();
    let _ = std::fs::remove_file(&path);
    let t = Arc::new(ArrayTable::create(&path, TableKind::Confirmed).unwrap());
    t.set(1023, 7).unwrap();
    assert!(t.l2_resident_bytes() > 0, "test needs the in-RAM image");
    let stop = Arc::new(AtomicBool::new(false));
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let (t, stop) = (Arc::clone(&t), Arc::clone(&stop));
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let v = t.get(1023).unwrap();
                    assert!(v == 0 || v == 7, "torn read {v}");
                }
            })
        })
        .collect();
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    while std::time::Instant::now() < deadline {
        t.truncate(0).unwrap();
        t.set(1023, 7).unwrap();
    }
    stop.store(true, Ordering::Relaxed);
    for r in readers {
        assert!(r.join().is_ok(), "get panicked racing truncate");
    }
    let _ = std::fs::remove_file(&path);
}

/// Flush persists the live slots of the dirty range recorded at start.
/// A store from the write hook is refused; the bytes on disk are the slots
/// after that hook, so the flush did not keep a detached image. A store
/// after flush returns is absent here and present in the next flush.
#[test]
fn flush_writes_live_slots_and_refuses_a_store_until_it_finishes() {
    let path = tmp_path();
    let _ = std::fs::remove_file(&path);
    let t = ArrayTable::create(&path, TableKind::ArrayLink).unwrap();
    let n = 600u64;
    let pairs: Vec<(u64, u64)> = (0..n).map(|i| (i, i + 1)).collect();
    t.set_many(&pairs).unwrap();
    t.flush().unwrap();
    t.set(0, 5).unwrap();
    let mut during = Ok(());
    t.flush_dirty_between(|t, step| {
        if step == FlushStep::Write {
            if let Some(v) = t.data.write().unwrap_or_else(|e| e.into_inner()).as_mut() {
                v[0] = 77;
            }
            during = t.set(1, 9);
            let _ = t.set_many(&[(2, 8)]);
            let _ = t.truncate(n - 1);
        }
    })
    .unwrap();
    let slot0 = {
        let mut buf = [0u8; 8];
        t.file.read_at(FILE_HEADER_LEN as u64, &mut buf).unwrap();
        u64::from_le_bytes(buf)
    };
    assert!(
        during.is_err() && slot0 == 77,
        "store during flush refused={during:?} live slot0={slot0}"
    );
    assert!(n * ELEM > FLUSH_SCRATCH as u64);
    let chunk = t.flush_chunk_max.load(Ordering::Relaxed);
    assert!(chunk > 0 && chunk <= FLUSH_SCRATCH, "flush chunk {chunk}");
    t.set(1, 9).unwrap();
    let mut buf = [0u8; 8];
    t.file
        .read_at(FILE_HEADER_LEN as u64 + 8, &mut buf)
        .unwrap();
    assert_eq!(
        u64::from_le_bytes(buf),
        2,
        "store after flush stays off disk"
    );
    t.flush().unwrap();
    t.file
        .read_at(FILE_HEADER_LEN as u64 + 8, &mut buf)
        .unwrap();
    assert_eq!(u64::from_le_bytes(buf), 9);
    let _ = std::fs::remove_file(&path);
}

/// A rewrite whose length equals the last flushed length still publishes
/// that length. A torn longer HWM must not survive the flush.
#[test]
fn same_length_rewrite_publishes_logical_len() {
    let path = tmp_path();
    let _ = std::fs::remove_file(&path);
    let t = ArrayTable::create(&path, TableKind::Confirmed).unwrap();
    t.set_many(&[(0, 1), (1, 2), (2, 3), (3, 4)]).unwrap();
    t.flush().unwrap();
    let published = FILE_HEADER_LEN as u64 + 4 * ELEM;
    assert_eq!(t.file.logical_len(), published);
    t.file.set_logical_len(published + 64).unwrap();
    t.set(1, 99).unwrap();
    assert_eq!(t.len(), 4);
    assert_eq!(t.disk_len.load(Ordering::Acquire), 4);
    t.flush().unwrap();
    assert_eq!(t.file.logical_len(), published);
    drop(t);
    let t = ArrayTable::open(&path, TableKind::Confirmed).unwrap();
    assert_eq!(t.len(), 4);
    assert_eq!(t.get(1).unwrap(), 99);
    let _ = std::fs::remove_file(&path);
}

/// A failed `write_at` leaves the recorded range dirty. The retry writes
/// those slots and does not truncate the table.
#[cfg(unix)]
#[test]
fn failed_write_keeps_the_range_dirty() {
    let path = tmp_path();
    let _ = std::fs::remove_file(&path);
    let t = ArrayTable::create(&path, TableKind::ArrayLink).unwrap();
    t.set_many(&[(0, 1), (1, 2), (2, 3), (3, 4)]).unwrap();
    t.flush().unwrap();
    t.set(1, 42).unwrap();
    let mut swapped = None;
    let failed = t.flush_dirty_between(|t, step| {
        if step == FlushStep::Write {
            swapped = Some(crate::dev_full::DevFull::on(&t.file.read_file));
        }
    });
    drop(swapped);
    assert!(
        failed.is_err(),
        "write_at must fail while the fd is /dev/full"
    );
    assert_eq!(t.len(), 4, "a failed persist must not truncate");
    let mut buf = [0u8; 8];
    t.file
        .read_at(FILE_HEADER_LEN as u64 + 8, &mut buf)
        .unwrap();
    assert_eq!(
        u64::from_le_bytes(buf),
        2,
        "failed persist must not publish the slot"
    );
    t.flush().unwrap();
    t.file
        .read_at(FILE_HEADER_LEN as u64 + 8, &mut buf)
        .unwrap();
    assert_eq!(u64::from_le_bytes(buf), 42);
    let _ = std::fs::remove_file(&path);
}

/// A store during the clean step is refused. The store after flush returns
/// is what the next flush persists.
#[test]
fn set_after_flush_is_flushed_next() {
    let path = tmp_path();
    let _ = std::fs::remove_file(&path);
    let t = ArrayTable::create(&path, TableKind::ArrayLink).unwrap();
    t.set_many(&[(0, 10), (1, 11), (2, 12), (3, 13)]).unwrap();
    t.flush().unwrap();
    t.set(3, 23).unwrap();
    let mut during = Ok(());
    t.flush_dirty_between(|t, step| {
        if step == FlushStep::Clean {
            during = t.set(1, 21);
        }
    })
    .unwrap();
    assert!(during.is_err(), "store during clean must be refused");
    drop(t);
    let t = ArrayTable::open(&path, TableKind::ArrayLink).unwrap();
    assert_eq!((t.get(1).unwrap(), t.get(3).unwrap()), (11, 23));
    t.set(1, 21).unwrap();
    t.flush().unwrap();
    drop(t);
    let t = ArrayTable::open(&path, TableKind::ArrayLink).unwrap();
    assert_eq!((t.get(1).unwrap(), t.get(3).unwrap()), (21, 23));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn array_set_get_grow_set_many_truncate_flush() {
    let path = tmp_path();
    let _ = std::fs::remove_file(&path);
    let t = ArrayTable::create(&path, TableKind::Confirmed).unwrap();
    assert_eq!(t.len(), 0);
    assert_eq!(t.get(0).unwrap(), 0);
    t.set(3, 30).unwrap();
    assert_eq!(t.len(), 4);
    assert_eq!(t.get(0).unwrap(), 0);
    assert_eq!(t.get(3).unwrap(), 30);
    t.set(1, 11).unwrap();
    assert_eq!(t.get(1).unwrap(), 11);
    t.set_many(&[]).unwrap();
    t.set_many(&[(10, 100), (5, 50)]).unwrap();
    assert_eq!(t.len(), 11);
    assert_eq!(t.get(5).unwrap(), 50);
    assert_eq!(t.get(10).unwrap(), 100);
    t.flush().unwrap();
    t.flush_async().unwrap();
    drop(t);
    let t = ArrayTable::open(&path, TableKind::Confirmed).unwrap();
    assert_eq!(t.len(), 11);
    assert_eq!(t.get(3).unwrap(), 30);
    t.truncate(4).unwrap();
    assert_eq!(t.len(), 4);
    assert!(matches!(t.truncate(99), Err(StoreError::Corrupt(_))));
    t.flush().unwrap();
    {
        let mut raw = std::fs::read(&path).unwrap();
        let bad = (FILE_HEADER_LEN as u64 + 3).to_le_bytes();
        raw[8..16].copy_from_slice(&bad);
        std::fs::write(&path, &raw).unwrap();
    }
    assert!(matches!(
        ArrayTable::open(&path, TableKind::Confirmed),
        Err(StoreError::Corrupt(_))
    ));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn write_behind_no_flush_reopen_keeps_old_image() {
    let path = tmp_path();
    let _ = std::fs::remove_file(&path);
    {
        let t = ArrayTable::create(&path, TableKind::Confirmed).unwrap();
        t.set_many(&[(0, 10), (1, 20)]).unwrap();
        t.flush().unwrap();
        t.set(0, 999).unwrap();
        t.set(2, 30).unwrap();
        assert_eq!(t.get(0).unwrap(), 999);
        assert_eq!(t.len(), 3);
    }
    let t = ArrayTable::open(&path, TableKind::Confirmed).unwrap();
    assert_eq!(t.len(), 2, "len must stay at last flushed image");
    assert_eq!(t.get(0).unwrap(), 10);
    assert_eq!(t.get(1).unwrap(), 20);
    assert_eq!(t.get(2).unwrap(), 0);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn write_behind_flush_reopen_sees_new_image() {
    let path = tmp_path();
    let _ = std::fs::remove_file(&path);
    {
        let t = ArrayTable::create(&path, TableKind::Confirmed).unwrap();
        t.set_many(&[(0, 1), (1, 2)]).unwrap();
        t.flush().unwrap();
        t.set_many(&[(0, 7), (1, 8), (2, 9)]).unwrap();
        t.flush().unwrap();
    }
    let t = ArrayTable::open(&path, TableKind::Confirmed).unwrap();
    assert_eq!(t.len(), 3);
    assert_eq!(t.get(0).unwrap(), 7);
    assert_eq!(t.get(1).unwrap(), 8);
    assert_eq!(t.get(2).unwrap(), 9);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn write_behind_truncate_flush_reopen() {
    let path = tmp_path();
    let _ = std::fs::remove_file(&path);
    {
        let t = ArrayTable::create(&path, TableKind::Confirmed).unwrap();
        t.set_many(&[(0, 1), (1, 2), (2, 3), (3, 4)]).unwrap();
        t.flush().unwrap();
        t.truncate(2).unwrap();
        assert_eq!(t.len(), 2);
        drop(t);
        let t = ArrayTable::open(&path, TableKind::Confirmed).unwrap();
        assert_eq!(t.len(), 4);
        t.truncate(2).unwrap();
        t.flush().unwrap();
    }
    let t = ArrayTable::open(&path, TableKind::Confirmed).unwrap();
    assert_eq!(t.len(), 2);
    assert_eq!(t.get(0).unwrap(), 1);
    assert_eq!(t.get(1).unwrap(), 2);
    let _ = std::fs::remove_file(&path);
}

/// Tip extension is append-only: after first flush, only suffix is written.
#[test]
fn append_only_flush_extends_without_losing_prefix() {
    let path = tmp_path();
    let _ = std::fs::remove_file(&path);
    {
        let t = ArrayTable::create(&path, TableKind::Confirmed).unwrap();
        t.set_many(&[(0, 100), (1, 101)]).unwrap();
        t.flush().unwrap();
        // Pure tip extension (unstored index 2, disk length 2).
        t.set(2, 102).unwrap();
        t.flush().unwrap();
    }
    let t = ArrayTable::open(&path, TableKind::Confirmed).unwrap();
    assert_eq!(t.len(), 3);
    assert_eq!(t.get(0).unwrap(), 100);
    assert_eq!(t.get(1).unwrap(), 101);
    assert_eq!(t.get(2).unwrap(), 102);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn set_gap_zeros_prefix_and_env_cap_and_flush_async() {
    let path = tmp_path();
    let _ = std::fs::remove_file(&path);
    let t = ArrayTable::create(&path, TableKind::Confirmed).unwrap();
    t.set(3, 42).unwrap();
    assert_eq!(t.len(), 4);
    assert_eq!(t.get(0).unwrap(), 0);
    assert_eq!(t.get(1).unwrap(), 0);
    assert_eq!(t.get(2).unwrap(), 0);
    assert_eq!(t.get(3).unwrap(), 42);
    t.set_many(&[(1, 11), (5, 55), (3, 33)]).unwrap();
    assert_eq!(t.len(), 6);
    assert_eq!(t.get(1).unwrap(), 11);
    assert_eq!(t.get(3).unwrap(), 33);
    assert_eq!(t.get(5).unwrap(), 55);
    t.flush_async().unwrap();
    t.flush_dirty().unwrap();
    t.flush().unwrap();
    t.truncate(0).unwrap();
    t.flush().unwrap();
    assert_eq!(t.len(), 0);
    drop(t);

    let prev = std::env::var_os("RBITCOIN_CLASS_C_INRAM_MAX_MB");
    std::env::remove_var("RBITCOIN_CLASS_C_INRAM_MAX_MB");
    assert_eq!(class_c_inram_max_bytes(), DEFAULT_CLASS_C_INRAM_MAX_BYTES);
    std::env::set_var("RBITCOIN_CLASS_C_INRAM_MAX_MB", "128");
    assert_eq!(class_c_inram_max_bytes(), 128 * 1024 * 1024);
    std::env::set_var("RBITCOIN_CLASS_C_INRAM_MAX_MB", "not-a-number");
    assert_eq!(class_c_inram_max_bytes(), 256 * 1024 * 1024); // parse fallback
    match prev {
        Some(v) => std::env::set_var("RBITCOIN_CLASS_C_INRAM_MAX_MB", v),
        None => std::env::remove_var("RBITCOIN_CLASS_C_INRAM_MAX_MB"),
    }
    let _ = std::fs::remove_file(&path);
}

/// A store from another thread while flush is in its write step is refused.
/// The store after that flush joins is what the next flush persists.
#[test]
fn store_on_another_thread_during_flush_is_refused() {
    use std::sync::Arc;
    let path = tmp_path();
    let _ = std::fs::remove_file(&path);
    let t = Arc::new(ArrayTable::create(&path, TableKind::Confirmed).unwrap());
    t.set_many(&[(0, 1), (1, 2)]).unwrap();
    t.flush().unwrap();
    t.set(0, 5).unwrap();
    let (started, start_rx) = std::sync::mpsc::channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let flush_t = {
        let t = Arc::clone(&t);
        std::thread::spawn(move || {
            t.flush_dirty_between(|_, step| {
                if step == FlushStep::Write {
                    started.send(()).unwrap();
                    release_rx.recv().unwrap();
                }
            })
            .unwrap();
        })
    };
    start_rx.recv().unwrap();
    let during = t.set(0, 9);
    release.send(()).unwrap();
    flush_t.join().unwrap();
    assert!(during.is_err(), "store during flush refused={during:?}");
    drop(t);
    let t = ArrayTable::open(&path, TableKind::Confirmed).unwrap();
    assert_eq!(t.get(0).unwrap(), 5);
    t.set(0, 9).unwrap();
    t.flush().unwrap();
    drop(t);
    let t = ArrayTable::open(&path, TableKind::Confirmed).unwrap();
    assert_eq!(t.get(0).unwrap(), 9);
    let _ = std::fs::remove_file(&path);
}
