Changed

- Linux CI and nightly mutants run test binaries with a private `TMPDIR`
  on `/dev/shm` (`scripts/tmpfs-test-runner.sh`). Store fsyncs no longer
  dominate suite wall time. The two `sp_tweaks` u32-roll tests no longer
  allocate 4–5 GB of disk each.
