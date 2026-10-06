Fixed

- **IBD retries a wave after a local store fault in load, scripts, or
  write.** Load offered the wave's bodies back but left lookup's consume
  mark past them. Scripts and write, and an io_uring session recover in any
  of the three, did not offer them back at all. Lookup never took those
  heights again unless a later wave, a disconnect, or a reorg re-armed it,
  so near the tip IBD stalled. Each stage now re-arms lookup at the tip and
  then offers the bodies back. The retry keeps the whole wave, except a
  pin-stage fault in a multi-block wave, which retries one block at a time.
