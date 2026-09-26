Changed

- **`--sp-tweaks` and `--prune-seqsigwit` are refused together.** Tweaks
  read input public keys from scriptSig and witness data, which pruning
  drops. The node now refuses the pair at startup, refuses to enable
  pruning while tweaks are on (and tweaks on a datadir that was pruned),
  and a pruned node answers tweak requests with an error instead of
  computing them from data it claims not to keep.
