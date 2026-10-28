//! `in_wave` is published before `next`. A publisher that sees
//! [`Wave::is_complete`] must not free `ctx` under a claim that still applies.

use rbitcoin_consensus::script_pool::{Apply, Wave};
use rbitcoin_consensus::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use rbitcoin_consensus::sync::thread;
use rbitcoin_consensus::ConsensusError;
use std::sync::Arc;

unsafe fn nop(_: *const (), _: usize) -> Result<(), ConsensusError> {
    Ok(())
}

#[test]
fn claim_holds_in_wave_before_next_advances() {
    super::engine::check(|| {
        let wave = Arc::new(Wave {
            n: 1,
            chunk: 1,
            next: AtomicUsize::new(0),
            in_wave: AtomicUsize::new(0),
            failed: std::sync::atomic::AtomicBool::new(false),
            first_err: std::sync::Mutex::new(None),
            keep_faults: std::sync::atomic::AtomicBool::new(false),
            faults: std::sync::Mutex::new(Vec::new()),
            apply: Apply {
                f: nop,
                ctx: std::ptr::null(),
            },
            done: std::sync::Mutex::new(false),
            done_cv: std::sync::Condvar::new(),
        });
        let freed = Arc::new(AtomicBool::new(false));
        let publishing = Arc::clone(&wave);
        let flag = Arc::clone(&freed);
        let publisher = thread::spawn(move || {
            if publishing.is_complete() {
                flag.store(true, Ordering::Release);
            }
        });
        let claiming = Arc::clone(&wave);
        let flag = Arc::clone(&freed);
        let claimer = thread::spawn(move || {
            if let Some((_, hold)) = Wave::claim_chunk(&claiming) {
                assert!(
                    !flag.load(Ordering::Acquire),
                    "is_complete freed ctx under an in-flight claim"
                );
                drop(hold);
            }
        });
        claimer.join().unwrap();
        publisher.join().unwrap();
    });
}
