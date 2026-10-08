Security

- **A lost spend-edge tail is no longer spendable after restart.** If the
  parent-edge bytes for confirmed outputs are short or zeroed, reopen does
  not publish those outputs as unspent. The tip moves back past the first
  such output, or the node refuses to start. A checkpoint that saw a
  disconnect does not publish through the replacement block at that height.
- **A peer that never reads cannot pin compact-block transaction serving.**
  `getblocktxn` waits on the same send budget as other served blocks, and
  `blocktxn` is charged by its real size. The reconstruct runs off the
  peer task.
- **Mempool eviction drops the relay indexes with the transaction.** Fee
  and slot-table eviction, expiry past a run of already-removed entries,
  and a rolled-back package or one-parent package clear the scripthash,
  expiry, and wtxid maps for every transaction that left, including one
  the rolled-back member had evicted. A prioritisation delta stays until
  the transaction is mined. A reorg removes a parent and its children
  before a template can select the child. A coin that is spent while its
  script is checked is not admitted. That recheck reads the coin before
  taking the mempool write lock, unless a block connect or disconnect is
  changing spentness.
- **Bad compact blocks and oversized transaction counts are refused before
  the expensive work.** Once the tip meets minimum chain work, a
  `cmpctblock` with bad proof of work is scored like a bad block. It does
  not scan the mempool or update that peer's header state. A `tx`,
  `block`, `cmpctblock`, or `blocktxn` whose witness count cannot fit in
  the payload is misbehavior. Those four relay messages are walked twice
  — inputs, outputs, and witnesses — before `consensus_decode`. IBD block
  frames skip that pre-walk. `merkleblock` is ignored.
- **Electrum mempool status no longer runs on the connection.** With
  Electrum enabled, a mempool payment or replacement of a subscribed
  scripthash still pushes the new status, and the history join is off the
  session task.
