Fixed

- **A txid parent is requested after the same peer's wtxid window ends.**
  Expiry matched the first announcement for that peer. When that row was
  the waiting txid parent, the in-flight wtxid window stayed indexed and
  no getdata followed.
