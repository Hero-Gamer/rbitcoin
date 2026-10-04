#![no_main]

use std::sync::atomic::{AtomicU64, Ordering};

use libfuzzer_sys::fuzz_target;
use rbitcoin_fuzz::{compare_kernel_bytes, KernelCmp};

static COMPARISONS: AtomicU64 = AtomicU64::new(0);

fn note_comparison() {
    let n = COMPARISONS.fetch_add(1, Ordering::Relaxed) + 1;
    if n == 1 || n.is_multiple_of(1000) {
        eprintln!("script-kernel: comparisons={n}");
    }
}

fuzz_target!(|data: &[u8]| {
    let Some(cmp) = compare_kernel_bytes(data) else {
        return;
    };
    match cmp {
        KernelCmp::Agree { .. } => note_comparison(),
        KernelCmp::Skip => {}
        KernelCmp::Disagree { ours, core } => {
            panic!("script_kernel: ours={ours} core={core} input={data:?}");
        }
    }
});
