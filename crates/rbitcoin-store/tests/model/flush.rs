//! Flush exclusion. A store that returns Ok is the slot on disk when that
//! flush left the mark clean. `get_sample` does not cover this handshake.

use rbitcoin_primitives::TableKind;
use rbitcoin_store::array_table::{ArrayTable, FlushStep};
use rbitcoin_store::ClassCWrite;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

#[test]
fn a_store_that_passes_the_flush_is_the_slot_on_disk() {
    super::engine::check_at_most(3, || {
        let dir = std::env::temp_dir().join("rbtc-flush-model");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let table =
            Arc::new(ArrayTable::create(dir.join("slots.body"), TableKind::ArrayLink).unwrap());
        table.set(&ClassCWrite::for_test(), 0, 5).unwrap();
        let entered = Arc::new(AtomicBool::new(false));
        let sampled = Arc::new(AtomicU64::new(u64::MAX));
        let writing = Arc::clone(&table);
        let entered_w = Arc::clone(&entered);
        let writer = loom::thread::spawn(move || {
            if writing.set(&ClassCWrite::for_test(), 0, 99).is_ok() {
                entered_w.store(true, Ordering::Relaxed);
            }
        });
        let flushing = Arc::clone(&table);
        let sampled_f = Arc::clone(&sampled);
        let flusher = loom::thread::spawn(move || {
            flushing
                .flush_dirty_between(|t, step| {
                    if step == FlushStep::Write {
                        sampled_f.store(t.live_slot(0), Ordering::Relaxed);
                    }
                })
                .unwrap();
        });
        writer.join().unwrap();
        flusher.join().unwrap();
        if entered.load(Ordering::Relaxed)
            && table.unstored_from.load(Ordering::Relaxed) == u64::MAX
        {
            let mut buf = [0u8; 8];
            table.file.read_at(ArrayTable::offset(0), &mut buf).unwrap();
            let disk = u64::from_le_bytes(buf);
            let seen = sampled.load(Ordering::Relaxed);
            if disk != 99 {
                panic!("committed store lost: disk={disk} mark=CLEAN entered=true sampled={seen}");
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    });
}
