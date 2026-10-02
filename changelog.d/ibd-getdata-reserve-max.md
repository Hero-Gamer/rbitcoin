Changed

- IBD prices a new getdata hash against the assign-stop at the largest
  wire length among the most recent 32 requested bodies, and never above
  4 MiB. Until eight bodies have been recorded the charge stays 4 MiB.
  A hash already in flight is still issued. The per-peer cap stays the
  median of recent bodies (1 KiB until eight), then 64 blocks or 16 MiB.
