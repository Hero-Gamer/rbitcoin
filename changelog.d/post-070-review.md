Fixed

- **Esplora `after_txid` is 422 when that tx is not in the script's history.**
  A cursor that exists somewhere else on the chain used to restart page 1.
  `/txs`, `/txs/chain`, `/txs/summary`, the address routes, and a multi
  POST now return `after_txid not found` and no rows.
- **`estimatesmartfee`, `estimaterawfee`, and Electrum `blockchain.estimatefee`
  use the 2-block rate for target 2.** Target 0 is still the next-block
  horizon. Target 1 stays the 1-block rate.
- **A shared orphan survives the other announcer's reserve.** Evicting
  peer B drops only B. Peer A's copy is still delivered when the parent
  arrives.
- **A tip shrink clamps the spend-durable marker.** Open revalidation and
  spend replay lower a marker that sits above the surviving tip, so a
  later confirm still annotates spends. A checkpoint cannot publish the
  pre-disconnect height over that clamp.
- **A newest-first scripthash page stops at the page edge.** An unspent
  tail no longer re-reads every older create.
- **A tip or compact block with a repeated transaction pair is not a
  block.** The merkle root can still match (CVE-2012-2459). Tip follow
  disconnects that peer. Compact reconstruction returns the hash to
  `getdata`.
- **`submitblock` of a sibling that spends a coin the tip also spent is
  inconclusive.** That header is not cached as `duplicate-invalid`.
- **A refused local I2P SAM port does not rotate the session.** The dial
  error is no longer classified as a dead `STREAM CONNECT`. A SAM reply
  of `INVALID_ID` still is.
- **Outbound dial keeps one onion or I2P seat when clearnet fills the
  batch.** A dead overlay peer is recorded on its real address. An
  unspecified version socket is not inserted into addrman.
- **An Esplora singleflight join does not put an older scripthash back
  over a newer last-1** for the same client. That includes a leader that
  is still inside its handler when the newer script finishes, and a waiter
  that resumes after it.
