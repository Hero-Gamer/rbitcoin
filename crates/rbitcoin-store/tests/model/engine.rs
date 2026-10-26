//! One `check` for loom. The bound is the PR budget. `LOOM_MAX_PREEMPTIONS`
//! overrides it. `MODEL_LOOM_EXHAUSTIVE=1` removes it.
//!
//! [`check_at_most`] caps a model. The sequence counter stays at bound 2.
//! Flush exclusion stays at bound 3. A higher nightly bound does not raise
//! either cap.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const DEFAULT_PREEMPTIONS: usize = 2;

pub fn check(f: impl Fn() + Send + Sync + 'static) {
    check_limited(None, f);
}

/// At most `cap` preemptions. A lower `LOOM_MAX_PREEMPTIONS` still wins.
pub fn check_at_most(cap: usize, f: impl Fn() + Send + Sync + 'static) {
    check_limited(Some(cap), f);
}

fn check_limited(cap: Option<usize>, f: impl Fn() + Send + Sync + 'static) {
    let mut builder = loom::model::Builder::new();
    builder.preemption_bound = match std::env::var("MODEL_LOOM_EXHAUSTIVE")
        .ok()
        .and_then(|v| v.parse::<u8>().ok())
    {
        Some(1) => None,
        _ => {
            let bound = builder.preemption_bound.or(Some(DEFAULT_PREEMPTIONS));
            match (bound, cap) {
                (Some(bound), Some(cap)) => Some(bound.min(cap)),
                (bound, _) => bound,
            }
        }
    };
    let runs = Arc::new(AtomicUsize::new(0));
    let start = std::time::Instant::now();
    let seen = Arc::clone(&runs);
    builder.check(move || {
        seen.fetch_add(1, Ordering::Relaxed);
        f();
    });
    eprintln!(
        "model: {} executions in {:.1?}",
        runs.load(Ordering::Relaxed),
        start.elapsed()
    );
}
