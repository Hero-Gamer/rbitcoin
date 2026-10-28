/// Points an open file descriptor at `/dev/full` until dropped.
#[cfg(target_os = "linux")]
pub struct DevFull {
    fd: std::os::fd::RawFd,
    saved: std::os::fd::RawFd,
}

#[cfg(target_os = "linux")]
impl DevFull {
    pub fn on(file: &std::fs::File) -> Self {
        use std::os::fd::AsRawFd;
        let fd = file.as_raw_fd();
        // SAFETY: `fd` belongs to `file` for this call. `dup` returns a new descriptor or -1.
        let saved = unsafe { libc::dup(fd) };
        assert!(saved >= 0, "dup");
        let full = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/full")
            .unwrap();
        // SAFETY: `full` is open. `dup2` replaces `fd` in place; `Drop` puts the saved descriptor back.
        let rc = unsafe { libc::dup2(full.as_raw_fd(), fd) };
        assert_eq!(rc, fd, "dup2");
        Self { fd, saved }
    }
}

#[cfg(target_os = "linux")]
impl Drop for DevFull {
    fn drop(&mut self) {
        // SAFETY: `saved` is the duplicate from `on`. Restore `fd`, then close that duplicate.
        unsafe {
            libc::dup2(self.saved, self.fd);
            libc::close(self.saved);
        }
    }
}
