//! `std::sync::atomic` behind one import. `--cfg loom` swaps in loom's
//! atomics so a model can see Class C sequence counters and the UTXO view
//! word. Production is `std`. Shuttle does not rebuild this crate's atomics:
//! its model is the script-pool idle turn.

#[cfg(loom)]
pub use loom::sync::atomic;

#[cfg(not(loom))]
pub use std::sync::atomic;

/// Pause a reader while a writer is in the odd section. A model has to yield
/// the thread here. Production is a processor hint.
#[cfg(loom)]
pub fn spin() {
    atomic::spin_loop_hint();
}

#[cfg(not(loom))]
pub fn spin() {
    std::hint::spin_loop();
}
