Changed

- **Far-target fee estimates answer right after startup.** Block fee
  history (used by 144/504/1008-block and blended mid targets) is read
  from the chain instead of a RAM ring filled only by this pool's own
  confirmed txs. When relay turns on, the node backfills the newest 1008
  blocks from stored fee rows and spend slots (no tx bodies); each new
  block is read the same way. A block counts even if this node never saw
  its txs.
- **Block fee history uses package rates.** Each tx counts at the rate
  of the ancestor set it was mined with, so a CPFP parent is not a
  zero-fee sample and a cheap child does not pull its parent down. Rates
  below min relay (out-of-band or zero-fee inclusions) are dropped before
  each block's p10.
