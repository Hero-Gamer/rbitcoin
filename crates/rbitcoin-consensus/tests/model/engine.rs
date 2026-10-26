//! Loom and shuttle budgets. Nightly raises them through the environment
//! (`.github/workflows/model.yml`).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[cfg(loom)]
const DEFAULT_PREEMPTIONS: usize = 2;

#[cfg(shuttle)]
const SEED: u64 = 0x5eed_a77a_7ab1_e5ee;
#[cfg(shuttle)]
const PCT_DEPTH: usize = 3;
#[cfg(shuttle)]
const PCT_ITERS: usize = 2_000;
#[cfg(shuttle)]
const RANDOM_ITERS: usize = 2_000;

#[cfg(loom)]
pub fn check(f: impl Fn() + Send + Sync + 'static) {
    let mut builder = loom::model::Builder::new();
    builder.preemption_bound = match std::env::var("MODEL_LOOM_EXHAUSTIVE")
        .ok()
        .and_then(|v| v.parse::<u8>().ok())
    {
        Some(1) => None,
        _ => builder.preemption_bound.or(Some(DEFAULT_PREEMPTIONS)),
    };
    counted(f, |f| builder.check(move || f()));
}

#[cfg(shuttle)]
pub fn check(f: impl Fn() + Send + Sync + 'static) {
    use shuttle::scheduler::{PctScheduler, RandomScheduler};
    let iters = std::env::var("MODEL_SHUTTLE_ITERS")
        .ok()
        .and_then(|v| v.parse().ok());
    let depth = std::env::var("MODEL_SHUTTLE_DEPTH")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(PCT_DEPTH);
    let seed = std::env::var("SHUTTLE_RANDOM_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(SEED);
    counted(f, |f| {
        let pct = Arc::clone(&f);
        shuttle::Runner::new(
            PctScheduler::new_from_seed(seed, depth, iters.unwrap_or(PCT_ITERS)),
            config(),
        )
        .run(move || pct());
        shuttle::Runner::new(
            RandomScheduler::new_from_seed(seed, iters.unwrap_or(RANDOM_ITERS)),
            config(),
        )
        .run(move || f());
    });
}

#[cfg(shuttle)]
fn config() -> shuttle::Config {
    let mut config = shuttle::Config::new();
    config.silence_warnings = true;
    config
}

fn counted<F: Fn() + Send + Sync + 'static>(
    f: F,
    explore: impl FnOnce(Arc<dyn Fn() + Send + Sync>),
) {
    let runs = Arc::new(AtomicUsize::new(0));
    let start = std::time::Instant::now();
    let seen = Arc::clone(&runs);
    explore(Arc::new(move || {
        seen.fetch_add(1, Ordering::Relaxed);
        f();
    }));
    eprintln!(
        "model: {} executions in {:.1?}",
        runs.load(Ordering::Relaxed),
        start.elapsed()
    );
}
