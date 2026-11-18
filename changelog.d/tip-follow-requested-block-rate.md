Fixed

- **Tip-follow no longer drops a peer for serving the blocks it asked for.**
  Every frame counted against the 16 MB/s per-peer rate window, including
  `block` replies to the node's own `getdata`. A fast peer that answered a
  full window of large blocks ended the session with `peer rate limit`, and
  an automatic outbound peer that did it twice in an hour was refused for a
  day. The first body for each request now stays off that window, as in
  IBD. A resent body, unsolicited bodies, and other messages still count.
