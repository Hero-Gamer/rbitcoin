Fixed

- A store whose `meta` is older than schema 22, empty or occupied, refuses
  with one wipe-and-IBD line before any table parser runs. Schema 22 and
  later still open, including the `create.loc.ovf` widen, the header
  size/weight rewrite, the inwit rename, and the `txstat` zero-extend.
