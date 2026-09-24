# 050 — Cluster limits rebuilt once per input

**Severity:** low
**Status:** fixed
**Found by:** @otaliptus (L-8)

`TxGraph::insert` called `cluster_of` (a full walk and linearize) once
per mempool input to find the cluster representative, and the pre-insert
limit check did the same once per parent.

Insert walks each touched component once. The cluster-build counter was
not a shipped surface and is gone. Cluster caps stay on
`mempool_under_pressure`.

**Regression:** `rbitcoin-mempool` `accept::tests::mempool_under_pressure`.
