//! IBD Class C barrier thread.
//!
//! The OS name is read from `/proc`, so this file is Linux-only. The count is
//! this query's thread: other tests in the process also name `rbtc-tip-flush`.

#![cfg(target_os = "linux")]

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// Two barriers share one `rbtc-tip-flush` thread. Tip mode does not start it.
#[test]
fn two_ibd_flushes_share_one_named_thread() {
    let (dir, q) = rbitcoin_query::testutil::tiny_query_labeled("tip-flush-one");
    assert!(q.index_mode().unwrap().is_tip());
    q.flush_tip(None).unwrap();
    assert_eq!(q.tip_flush_alive(), 0, "tip mode flushes inline");

    q.enter_direct_index_mode_sh(false).unwrap();
    let hold = q.hold_tip_flushes();
    q.spawn_tip_flush().unwrap();
    q.spawn_tip_flush().unwrap();
    let deadline = Instant::now() + Duration::from_millis(200);
    let mut tid = 0u64;
    while Instant::now() < deadline {
        let alive = q.tip_flush_alive();
        let published = q.tip_flush_os_tid();
        if tid == 0 {
            assert!(alive <= 1, "two IBD barriers use one rbtc-tip-flush thread");
            if alive == 1 && published != 0 {
                tid = published;
            }
        } else {
            assert_eq!(alive, 1, "two IBD barriers use one rbtc-tip-flush thread");
            assert_eq!(published, tid, "the second barrier starts another thread");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_ne!(tid, 0, "rbtc-tip-flush published its tid");
    assert_eq!(
        task_comm(tid).as_deref(),
        Some("rbtc-tip-flush"),
        "IBD barrier thread name"
    );
    hold.store(false, Ordering::SeqCst);
    q.join_tip_flush().unwrap();
    assert_eq!(
        q.tip_flush_alive(),
        1,
        "the barrier thread outlives one flush"
    );
    assert_eq!(q.tip_flush_os_tid(), tid);
    drop(q);
    drop(dir);
    assert!(task_comm(tid).is_none(), "drop joins rbtc-tip-flush");
}

fn task_comm(tid: u64) -> Option<String> {
    let raw = std::fs::read_to_string(format!("/proc/self/task/{tid}/comm")).ok()?;
    Some(raw.trim_end_matches('\n').to_string())
}
