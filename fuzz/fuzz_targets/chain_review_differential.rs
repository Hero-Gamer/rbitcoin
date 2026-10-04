#![no_main]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use rbitcoin_fuzz::{
    check_diff_env, compare_chain_plan, plan_chain_shape, spawn_bitcoind_extra, tmp_dir,
    ChainReview, CoreChild,
};

static CORE: OnceLock<CoreChild> = OnceLock::new();
static COMPARISONS: AtomicU64 = AtomicU64::new(0);
static ORACLE_DOWN_STREAK: AtomicU64 = AtomicU64::new(0);
const MAX_ORACLE_DOWN_STREAK: u64 = 20;

fn harness_failure(what: &str) -> ! {
    eprintln!("=== CHAIN-REVIEW FUZZ HARNESS FAILURE ===");
    eprintln!("{what}");
    eprintln!(
        "comparisons_before_failure={}",
        COMPARISONS.load(Ordering::Relaxed)
    );
    std::process::exit(2);
}

fn note_comparison() {
    ORACLE_DOWN_STREAK.store(0, Ordering::Relaxed);
    let n = COMPARISONS.fetch_add(1, Ordering::Relaxed) + 1;
    if n == 1 || n.is_multiple_of(100) {
        eprintln!("chain-review: comparisons={n}");
    }
}

fn core() -> &'static CoreChild {
    CORE.get_or_init(|| {
        let io = std::env::var("RBITCOIN_IO").ok();
        if let Err(e) = check_diff_env(None, io.as_deref()) {
            harness_failure(e);
        }
        let bin = std::env::var("RBITCOIN_CORE_BITCOIND").unwrap_or_default();
        if bin.is_empty() {
            harness_failure("RBITCOIN_CORE_BITCOIND unset");
        }
        let dir = tmp_dir("rbtc-chain-review-core");
        // Same BIP34-off regtest the hub shapes use, so a duplicate coinbase is
        // not rejected as a height push before BIP30.
        spawn_bitcoind_extra(
            std::path::Path::new(&bin),
            &dir,
            &["-testactivationheight=bip34@100000000"],
        )
        .unwrap_or_else(|e| harness_failure(&e))
    })
}

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }
    let shape = data[0];
    let plan = match plan_chain_shape(shape) {
        Ok(plan) => plan,
        Err(e) => harness_failure(&e),
    };
    match compare_chain_plan(&plan, &core().rpc) {
        Ok(ChainReview::Agree { .. }) => note_comparison(),
        Ok(ChainReview::Disagree { ours, core }) => {
            eprintln!("=== CHAIN-REVIEW FUZZ CONSENSUS DIVERGENCE ===");
            eprintln!("ours_accept={ours} core_accept={core} shape={shape}");
            panic!("chain-review: ours={ours} core={core} shape={shape}");
        }
        Err(msg) if msg == "oracle dead" || msg == "oracle rpc" => {
            let n = ORACLE_DOWN_STREAK.fetch_add(1, Ordering::Relaxed) + 1;
            if n >= MAX_ORACLE_DOWN_STREAK {
                harness_failure(&msg);
            }
        }
        Err(msg) => harness_failure(&msg),
    }
});
