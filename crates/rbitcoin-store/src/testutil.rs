//! Drop-cleaning Tiny fixtures for tests. Not an operator API.
//!
//! Production constructors stay Mainnet (`StoreLayout::single`). Tests that
//! need an on-disk store use [`tiny_store`] / [`TempDir`] instead of rolling
//! `std::env::temp_dir()` + `create_dir_all`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::Store;

static SEQ: AtomicU64 = AtomicU64::new(0);

/// Unique directory removed on drop.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new() -> std::io::Result<Self> {
        Self::labeled("tmp")
    }

    pub fn labeled(label: &str) -> std::io::Result<Self> {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let safe: String = label
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        let path = std::env::temp_dir().join(format!(
            "rbitcoin-{safe}-{}-{}-{}",
            std::process::id(),
            nanos,
            n
        ));
        std::fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn as_path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

impl AsRef<Path> for TempDir {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<std::ffi::OsStr> for TempDir {
    fn as_ref(&self) -> &std::ffi::OsStr {
        self.path.as_os_str()
    }
}

impl std::ops::Deref for TempDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}

/// Tiny-head store in a drop-cleaning directory.
pub fn tiny_store() -> (TempDir, Store) {
    tiny_store_labeled("store")
}

pub fn tiny_store_labeled(label: &str) -> (TempDir, Store) {
    let dir = TempDir::labeled(label).expect("create temp dir");
    let store = Store::create_tiny(dir.path()).expect("create tiny store");
    (dir, store)
}

/// Tiny store at `dir` whose `tx.head` segments have `2^bits` slots, so a
/// few hundred creates fill one and roll it.
pub fn create_tiny_with_head_bits(dir: &Path, bits: u32) -> Store {
    let head = crate::address_head::HeadLayout::new(bits).expect("head bits in range");
    Store::create_layout_with_head(crate::StoreLayout::tiny(dir.to_path_buf()), head)
        .expect("create tiny store")
}

/// Zero the slot pages of every unsealed `tx.head` segment under `root`,
/// keeping each file's length, its trailing layout footer, and the synced
/// `meta`. A power loss leaves this when `meta` was synced after an insert
/// and the page writes were not.
pub fn drop_unsynced_tx_head_pages(root: &Path) {
    fn find_head(dir: &Path) -> Option<PathBuf> {
        std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
            let p = e.path();
            if !p.is_dir() {
                return None;
            }
            if p.file_name()? == "tx.head" {
                Some(p)
            } else {
                find_head(&p)
            }
        })
    }
    let head = find_head(root).expect("tx.head under root");
    for e in std::fs::read_dir(&head).expect("read tx.head").flatten() {
        let p = e.path();
        if p.extension().is_some() || p.file_name().is_some_and(|n| n == "meta") {
            continue;
        }
        let len = std::fs::metadata(&p).expect("segment len").len();
        let slots = len.saturating_sub(crate::file::TRAILING_FOOTER_LEN as u64);
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .open(&p)
            .expect("open segment");
        std::io::Write::write_all(&mut f, &vec![0u8; slots as usize]).expect("zero slot pages");
    }
}
