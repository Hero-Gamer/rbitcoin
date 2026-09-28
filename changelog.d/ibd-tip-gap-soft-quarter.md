Fixed

- IBD treats a tip gap as a hole once the body queue meets any of: a quarter
  of the confirm-time block window, a quarter of the configured assign-stop
  (default 1 GiB), or 1000 blocks. Below all three the gap is the frontier:
  tip+1 gets one peer and densify keeps filling ahead. Gaps in the queue
  count. The 100 MiB free floor remains the densify horizon only.
