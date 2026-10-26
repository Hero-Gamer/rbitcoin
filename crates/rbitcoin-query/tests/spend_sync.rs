use std::sync::Arc;

use rbitcoin_primitives::{Fk, Height};

use rbitcoin_query::spend_sync::SpendSync;

#[test]
fn shutdown_publishes_the_snapshot() {
    let (dir, q) = rbitcoin_query::testutil::tiny_query_labeled("spend-sync-stop");
    let pairs: Vec<_> = (0..=4u32).map(|h| (Height(h), Fk(1))).collect();
    q.store()
        .confirmed
        .set_many(&rbitcoin_store::ClassCWrite::for_test(), &pairs)
        .unwrap();
    q.store().note_spend_snapshot(4);
    let q = Arc::new(q);
    let sync = SpendSync::spawn(Arc::clone(&q));
    sync.shutdown();
    let raw = std::fs::read(q.store().path().join(rbitcoin_store::SPEND_DURABLE_NAME)).unwrap();
    assert_eq!(u32::from_le_bytes(raw[8..12].try_into().unwrap()), 4);
    assert_eq!(u32::from_le_bytes(raw[12..16].try_into().unwrap()), 4);
    let _ = dir;
}
