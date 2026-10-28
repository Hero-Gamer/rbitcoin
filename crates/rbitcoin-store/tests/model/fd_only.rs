//! Disk-only Class C. An append's length covers that store's slot. A get
//! that races a truncate reads as absent.
//!
//! The file is pre-sized so `open` stays fd-only under a zero RAM cap. An
//! empty `create` is in RAM even when that cap is 0. Index 0 of a one-slot
//! file is an overwrite. The append is the next index.

use rbitcoin_primitives::TableKind;
use rbitcoin_store::array_table::ArrayTable;
use rbitcoin_store::file::TableFile;
use rbitcoin_store::ClassCWrite;
use std::path::Path;

fn open_fd_only(dir: &Path, slots: u64) -> ArrayTable {
    let path = dir.join("slots.body");
    let file = TableFile::create(&path, TableKind::ArrayLink).unwrap();
    let zeros = vec![0u8; (slots * 8) as usize];
    if slots > 0 {
        file.write_at(ArrayTable::offset(0), &zeros).unwrap();
    }
    drop(file);
    let prev = std::env::var_os("RBITCOIN_CLASS_C_INRAM_MAX_MB");
    std::env::set_var("RBITCOIN_CLASS_C_INRAM_MAX_MB", "0");
    let table = ArrayTable::open(&path, TableKind::ArrayLink).unwrap();
    match prev {
        Some(v) => std::env::set_var("RBITCOIN_CLASS_C_INRAM_MAX_MB", v),
        None => std::env::remove_var("RBITCOIN_CLASS_C_INRAM_MAX_MB"),
    }
    assert_eq!(table.l2_resident_bytes(), 0, "disk-only table");
    table
}

/// One append against one sample. A length that covers the new index has
/// that store's value.
#[test]
fn fd_only_append_publishes_its_value() {
    super::engine::check_at_most(2, || {
        let dir = std::env::temp_dir().join("rbtc-fd-append");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let slots = std::sync::Arc::new(open_fd_only(&dir, 1));
        let slots_w = std::sync::Arc::clone(&slots);
        let writer = loom::thread::spawn(move || {
            slots_w.set(&ClassCWrite::for_test(), 1, 99).unwrap();
        });
        let reader = loom::thread::spawn(move || {
            let (value, len) = slots.get_sample(1).unwrap();
            if len > 1 {
                assert_eq!(value, 99, "new length with a zero fill");
            }
        });
        writer.join().unwrap();
        reader.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    });
}
