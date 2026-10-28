//! UTXO view stamp. While [`Store::hold_utxo_view`] is held the word is odd
//! and [`Store::utxo_view_stamp`] is `None`.

#[test]
fn stamp_is_absent_while_the_view_is_held() {
    super::engine::check(|| {
        let dir = std::env::temp_dir().join("rbtc-utxo-model");
        let _ = std::fs::remove_dir_all(&dir);
        // `create_tiny` does not fit the model's default stack. The view word
        // is still minted inside the model, on this thread.
        let made = dir.clone();
        let store = loom::thread::Builder::new()
            .stack_size(8 * 1024 * 1024)
            .spawn(move || std::sync::Arc::new(rbitcoin_store::Store::create_tiny(&made).unwrap()))
            .unwrap()
            .join()
            .unwrap();
        let held = std::sync::Arc::clone(&store);
        let holder = loom::thread::spawn(move || {
            let guard = held.hold_utxo_view();
            assert!(
                held.utxo_view_stamp().is_none(),
                "stamp published an even word during the hold"
            );
            drop(guard);
        });
        let reader = loom::thread::spawn(move || {
            if let Some(stamp) = store.utxo_view_stamp() {
                assert_eq!(stamp & 1, 0, "odd stamp returned as a generation");
            }
        });
        holder.join().unwrap();
        reader.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    });
}
