//! Marks an `rbtc-scripts-*` worker. Store IO checks the flag and panics.

use std::cell::Cell;

thread_local! {
    static ON_SCRIPT_WORKER: Cell<bool> = const { Cell::new(false) };
}

/// This thread is an `rbtc-scripts-*` worker for the rest of its life.
#[doc(hidden)]
pub fn set_script_worker(on: bool) {
    ON_SCRIPT_WORKER.with(|c| c.set(on));
}

#[doc(hidden)]
pub fn on_script_worker() -> bool {
    ON_SCRIPT_WORKER.with(|c| c.get())
}

/// Store IO and other blocking work must not run on `rbtc-scripts-*`.
#[doc(hidden)]
pub fn assert_not_script_worker(what: &'static str) {
    assert!(!on_script_worker(), "{what} on rbtc-scripts");
}
