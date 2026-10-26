//! IBD Class C barrier thread.
//!
//! The OS name is read from `/proc`, so this file is Linux-only.

#![cfg(target_os = "linux")]

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// Two barriers share one `rbtc-tip-flush` thread. Tip mode does not start it.
#[test]
fn two_ibd_flushes_share_one_named_thread() {
    let (dir, q) = rbitcoin_query::testutil::tiny_query_labeled("tip-flush-one");
    assert!(q.index_mode().unwrap().is_tip());
    q.flush_tip(None).unwrap();
    assert_eq!(comm_now("rbtc-tip-flush"), 0, "tip mode flushes inline");

    q.enter_direct_index_mode_sh(false).unwrap();
    let hold = q.hold_tip_flushes();
    q.spawn_tip_flush().unwrap();
    q.spawn_tip_flush().unwrap();
    let n = comm_max("rbtc-tip-flush", Duration::from_millis(200));
    hold.store(false, Ordering::SeqCst);
    q.join_tip_flush().unwrap();
    drop(q);
    drop(dir);
    assert_eq!(n, 1, "two IBD barriers use one rbtc-tip-flush thread");
    assert_eq!(comm_now("rbtc-tip-flush"), 0, "drop joins rbtc-tip-flush");
}

fn comm_now(want: &str) -> usize {
    linux_comms()
        .iter()
        .filter(|name| name.as_str() == want)
        .count()
}

fn comm_max(want: &str, window: Duration) -> usize {
    let deadline = Instant::now() + window;
    let mut n = 0;
    loop {
        n = n.max(comm_now(want));
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    n
}

fn linux_comms() -> Vec<String> {
    let mut names = Vec::new();
    let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
        return names;
    };
    for task in tasks.flatten() {
        if let Ok(raw) = std::fs::read_to_string(task.path().join("comm")) {
            names.push(raw.trim_end_matches('\n').to_string());
        }
    }
    names
}
