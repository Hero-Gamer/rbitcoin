Changed

- During IBD a peer may have 64 blocks in flight, or 16 MiB of estimated
  block payload, whichever is reached first. The estimate is the median
  size of recent bodies, and 1 KiB until eight bodies have arrived, so
  early small blocks fill the count. The 1,024-block download window is
  unchanged.
