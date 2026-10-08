Fixed

- **Operator schema upgrade matches schema 26.** Opening a schema 24 or 25
  datadir shrinks `header.body` from 96 B to 88 B and rewrites `meta` to 26.
  The operator table no longer describes a rewrite that stops at 25 or grows
  headers from 88 B to 96 B.
- **Finding 085 is closed on IBD.** A coinbase-less 64-byte body is a bad
  copy: IBD drops it and asks again, and does not cache the hash as invalid.
