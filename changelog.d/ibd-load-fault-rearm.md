Fixed

- **IBD retries a wave after a local store fault in load.** The wave's
  bodies went back on the body queue, but lookup kept its consume mark past
  them and never took them again. Near the tip nothing else re-armed lookup,
  so IBD stalled. Load now re-arms lookup at the tip before the bodies go
  back, without isolating the retry.
