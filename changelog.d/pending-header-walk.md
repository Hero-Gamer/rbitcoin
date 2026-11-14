Security

- **A peer's parked headers are walked once per message, not once per
  item.** Headers that do not connect stay in the session's pending map (up
  to 8,000). An `inv` scored each block item, and a `headers` message each
  header, by walking that peer's parked chain toward our chain with a proof
  of work check per step. One `inv` of 50,000 items naming the tip of a
  2,000-header parked chain took about 20 seconds of node CPU, and thirty
  peers could stall every other peer and RPC for half a minute. The same
  per-leaf walk ran when a peer sent a block we already had, sent a compact
  block while the node is under `-minimumchainwork`, or sent a connecting
  header that needed room in a full pending map. Each message now scores
  every parked header at most once, so its cost is its items plus the parked
  headers. Parked leaves above headers the store already holds also share
  one walk of those while a pass has cached fewer than 10,001 entries for
  stored headers; past that, and above a stored trunk longer than the walk
  limit, each leaf walks them again as before. A header that does not connect no longer reads the header
  store once per parked ancestor. Best-known blocks and block requests are
  unchanged.
