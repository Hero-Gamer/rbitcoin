Changed

- **Store open unlinks `scripthash.runs` leftovers except `SEAL`.** The
  sorted-run writer and header parser are gone. Tip materialize does not
  write those files. A missing runs directory still opens. Open fails if a
  leftover cannot be read or removed. Resume a cancelled scripthash
  materialize from `scripthash.unsorted` or `scripthash.cold_progress`.
