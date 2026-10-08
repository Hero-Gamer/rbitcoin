Fixed

- **A package is trimmed once, after every member is in.** A parent that is
  under the fee floor alone is not evicted before its paying child commits,
  so `submitpackage` does not answer `mempool full` for a package that fits.
