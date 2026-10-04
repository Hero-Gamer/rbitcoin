Changed

- The nightly mutants job examines one source file per invocation, so
  struct-field deletes are not retested across the whole workspace.
  The checked-in mutants snapshot is gone; the night's artifact is the
  miss list.
