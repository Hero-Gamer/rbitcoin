//! Class C sequence counter. One `get_sample` is one generation: a length that
//! covers the index has that store's slot. One `get_range` is one header pair.

use rbitcoin_primitives::{Fk, TableKind};
use rbitcoin_store::chain::HeaderTxsTable;
use rbitcoin_store::ClassCWrite;

#[test]
fn len_belongs_to_the_slot_and_a_header_pair_does_not_mix() {
    super::engine::check_at_most(2, || {
        let dir = std::env::temp_dir().join("rbtc-seq-model");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let slots = std::sync::Arc::new(
            rbitcoin_store::array_table::ArrayTable::create(
                dir.join("slots.body"),
                TableKind::ArrayLink,
            )
            .unwrap(),
        );
        let headers = std::sync::Arc::new(HeaderTxsTable::create(&dir).unwrap());
        let slots_w = std::sync::Arc::clone(&slots);
        let headers_w = std::sync::Arc::clone(&headers);
        let writer = loom::thread::spawn(move || {
            let write = ClassCWrite::for_test();
            slots_w.set(&write, 3, 99).unwrap();
            headers_w.put_range(&write, Fk(1), Fk(10), 4).unwrap();
            headers_w.put_range(&write, Fk(1), Fk(50), 9).unwrap();
        });
        let reader = loom::thread::spawn(move || {
            let (value, len) = slots.get_sample(3).unwrap();
            if len > 3 {
                assert_eq!(value, 99, "new length with a slot from before the store");
            } else {
                assert_eq!(value, 0);
            }
            match headers.get_range(Fk(1)).unwrap() {
                None | Some((Fk(10), 4)) | Some((Fk(50), 9)) => {}
                other => panic!("torn header range {other:?}"),
            }
        });
        writer.join().unwrap();
        reader.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    });
}
