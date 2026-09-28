Fixed

- **A headers batch whose first header does not connect skips the longer
  prefix search.** After the full batch failed, IBD tried about ten longer
  prefixes even when the first header could not be stored. The search now
  stops after that first header fails. A connected first header still
  binary-searches the rest of the batch.
