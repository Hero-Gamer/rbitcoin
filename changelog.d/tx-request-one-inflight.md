Fixed

- **One peer is asked for an announced transaction.** A second
  announcement of the same wtxid or txid waits. Disconnect or `notfound`
  from the peer that was asked makes the waiting peer due. An inbound
  announcement is still requested immediately.
