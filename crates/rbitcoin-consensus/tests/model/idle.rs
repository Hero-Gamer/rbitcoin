//! Idle turn: register, recheck, park. A publisher that bumps the epoch
//! either is observed or unparks the registered worker. This does not start
//! the OS pool.

use rbitcoin_consensus::script_pool::{park_if_still_idle, unpark_idle_from, ScriptWorkers};
use rbitcoin_consensus::sync::atomic::{AtomicUsize, Ordering};
use rbitcoin_consensus::sync::{thread, Mutex};
use std::collections::VecDeque;
use std::sync::Arc;

#[test]
fn idle_turn_observes_the_epoch_or_wakes() {
    super::engine::check(|| {
        let pool = Arc::new(ScriptWorkers {
            jobs: Mutex::new(VecDeque::new()),
            epoch: AtomicUsize::new(0),
        });
        let idle = Arc::new(Mutex::new(Vec::new()));
        let pool_p = Arc::clone(&pool);
        let idle_p = Arc::clone(&idle);
        let publisher = thread::spawn(move || {
            pool_p.epoch.fetch_add(1, Ordering::Release);
            unpark_idle_from(&idle_p, 1);
        });
        let worker = thread::spawn(move || {
            let _ = park_if_still_idle(&pool, 0, &idle);
        });
        publisher.join().unwrap();
        worker.join().unwrap();
    });
}
