Fixed

- **`getblocktemplate` fees come from the selection.** Each
  transaction's `fee` and `sigops`, and the `coinbasevalue`, are read
  under the same mempool lock that selected it. A transaction evicted
  while the template was built no longer reports `fee: 0` and
  understates `coinbasevalue`.
