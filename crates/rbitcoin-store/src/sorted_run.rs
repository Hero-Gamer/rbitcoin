//! Worker cap from free RAM. Used by `tx.head` rebuild and scripthash extract.
//!
//! The sorted-run codec is gone. Tip materialize does not write `scripthash.runs`.
//! Store open unlinks leftover names in that directory except `SEAL`.

/// Cap workers at 1 per `per_worker` free bytes (floor 1, clamp 1..=256 vs CPUs).
pub fn workers_for_free_ram(cpus: usize, free_bytes: u64, per_worker: u64) -> usize {
    let cpus = cpus.clamp(1, 256);
    if per_worker == 0 {
        return 1;
    }
    let ram_cap = (free_bytes / per_worker) as usize;
    cpus.min(ram_cap.clamp(1, 256))
}

/// `MemAvailable` from `/proc/meminfo` text (kB → bytes).
#[cfg(any(test, target_os = "linux"))]
fn mem_available_from_meminfo(text: &str) -> Option<u64> {
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("MemAvailable:") else {
            continue;
        };
        let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
        return Some(kb.saturating_mul(1024));
    }
    None
}

/// Darwin reclaimable pages (`free_count + inactive_count`) × page size.
/// Speculative pages are already inside `free_count`.
#[cfg(any(test, target_os = "macos"))]
fn mem_available_from_darwin_vm(
    page_size: u64,
    free_count: u64,
    inactive_count: u64,
) -> Option<u64> {
    if page_size == 0 {
        return None;
    }
    Some(
        free_count
            .saturating_add(inactive_count)
            .saturating_mul(page_size),
    )
}

#[cfg(target_os = "macos")]
fn mem_available_from_darwin_host() -> Option<u64> {
    let page_size = {
        let n = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if n <= 0 {
            return None;
        }
        n as u64
    };
    let mut vm = unsafe { std::mem::zeroed::<libc::vm_statistics64>() };
    let mut count = libc::HOST_VM_INFO64_COUNT;
    let kr = unsafe {
        // SAFETY: process host port; HOST_VM_INFO64 writes into this stack vm_statistics64.
        #[allow(deprecated)]
        let host = libc::mach_host_self();
        libc::host_statistics64(
            host,
            libc::HOST_VM_INFO64,
            &mut vm as *mut _ as libc::host_info64_t,
            &mut count,
        )
    };
    if kr != libc::KERN_SUCCESS {
        return None;
    }
    mem_available_from_darwin_vm(
        page_size,
        u64::from(vm.free_count),
        u64::from(vm.inactive_count),
    )
}

#[cfg(windows)]
#[repr(C)]
#[allow(dead_code)]
struct MemoryStatusEx {
    dw_length: u32,
    dw_memory_load: u32,
    ull_total_phys: u64,
    ull_avail_phys: u64,
    ull_total_page_file: u64,
    ull_avail_page_file: u64,
    ull_total_virtual: u64,
    ull_avail_virtual: u64,
    ull_avail_extended_virtual: u64,
}

#[cfg(windows)]
extern "system" {
    fn GlobalMemoryStatusEx(status: *mut MemoryStatusEx) -> i32;
}

#[cfg(windows)]
fn mem_available_from_windows_host() -> Option<u64> {
    let mut st = MemoryStatusEx {
        dw_length: std::mem::size_of::<MemoryStatusEx>() as u32,
        dw_memory_load: 0,
        ull_total_phys: 0,
        ull_avail_phys: 0,
        ull_total_page_file: 0,
        ull_avail_page_file: 0,
        ull_total_virtual: 0,
        ull_avail_virtual: 0,
        ull_avail_extended_virtual: 0,
    };
    // SAFETY: dw_length is size_of Self; the API fills the remaining fields.
    if unsafe { GlobalMemoryStatusEx(&mut st) } == 0 {
        return None;
    }
    Some(st.ull_avail_phys)
}

/// Host memory available for new allocations.
///
/// Linux: `MemAvailable`. Darwin: free+inactive pages. Windows: `ullAvailPhys`.
/// Other OS: `None` (worker cap falls back to 1).
pub fn host_mem_available_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string("/proc/meminfo").ok()?;
        mem_available_from_meminfo(&text)
    }
    #[cfg(target_os = "macos")]
    {
        mem_available_from_darwin_host()
    }
    #[cfg(windows)]
    {
        return mem_available_from_windows_host();
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        None
    }
}

/// `MemAvailable` as a log label (`"12.3"` GiB, or `"?"` when unknown).
pub fn free_gib_label() -> String {
    host_mem_available_bytes()
        .map(|b| format!("{:.1}", b as f64 / (1u64 << 30) as f64))
        .unwrap_or_else(|| "?".into())
}

pub fn logical_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, 256)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workers_for_free_ram_1gib_head_is_not_sh_1_5gib() {
        const HEAD: u64 = crate::tx_table::TX_HEAD_REBUILD_WORKER_FREE_RAM_BYTES;
        assert_eq!(workers_for_free_ram(8, 0, HEAD), 1);
        assert_eq!(workers_for_free_ram(8, HEAD.saturating_sub(1), HEAD), 1);
        assert_eq!(workers_for_free_ram(8, HEAD, HEAD), 1);
        assert_eq!(workers_for_free_ram(8, 2 * HEAD, HEAD), 2);
        assert_eq!(workers_for_free_ram(8, 3 * HEAD, HEAD), 3);
        assert_eq!(workers_for_free_ram(4, 20 * HEAD, HEAD), 4);
        assert_eq!(workers_for_free_ram(8, 3 * (1 << 30) / 2, HEAD), 1);
        assert_eq!(workers_for_free_ram(8, 2 * (1 << 30), HEAD), 2);
    }

    #[test]
    fn mem_available_parses_proc_meminfo() {
        let text = "MemTotal:       16384000 kB\nMemFree:         1000000 kB\nMemAvailable:    3145728 kB\nBuffers:          200000 kB\n";
        assert_eq!(mem_available_from_meminfo(text), Some(3145728 * 1024));
        assert_eq!(mem_available_from_meminfo("MemTotal: 1 kB\n"), None);
    }

    #[test]
    fn darwin_vm_pages_count_as_free_ram() {
        assert_eq!(mem_available_from_darwin_vm(0, 10, 10), None);
        assert_eq!(mem_available_from_darwin_vm(4096, 0, 0), Some(0));
        assert_eq!(mem_available_from_darwin_vm(4096, 1, 0), Some(4096));
        assert_eq!(mem_available_from_darwin_vm(16384, 2, 2), Some(4 * 16384));
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    fn host_mem_available_bytes_is_some_on_linux_macos_windows() {
        let n =
            host_mem_available_bytes().expect("Linux MemAvailable / Darwin vm / Windows AvailPhys");
        assert!(n >= 1024 * 1024, "probe too small: {n}");
        let w = workers_for_free_ram(logical_cpus(), n, 2 << 30);
        assert!((1..=256).contains(&w));
        assert!(w <= logical_cpus());
        let label = free_gib_label();
        assert!(label == "?" || label.parse::<f64>().is_ok(), "{label}");
    }
}
