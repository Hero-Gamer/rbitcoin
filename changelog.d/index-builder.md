Changed

- **One builder for block filters and silent-payment tweaks.** After
  catch-up, a single background builder reads each window of blocks once
  (one completion session: io_uring, IOCP, or the macOS pool) and builds
  both `--block-filter-index` filters and `--sp-tweaks` tweaks from it.
  The two indexes no longer run separate passes over the same data.
  Progress logs as `index: build …`.
- **Tweaks no longer slow down block connection.** The confirm write thread
  no longer writes tweaks; the builder seals each new block after it is
  announced. `tip: accept` drops `tweaks=`.
- **A missing spent output is an error, not an ineligible tx.** Tweak
  computation used to skip a transaction whose spent output could not be
  found; it now reports store corruption.
