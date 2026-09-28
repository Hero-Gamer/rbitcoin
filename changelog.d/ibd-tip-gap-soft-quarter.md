Fixed

- IBD treats a tip gap as a hole only once the body queue is a quarter of
  the smaller soft budget (the confirm-time block window, or the 100 MiB
  free floor). Below that the gap is the frontier: tip+1 gets one peer and
  densify keeps filling ahead. Gaps in the queue count toward the quarter.
