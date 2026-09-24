# 050 — Cluster limits rebuilt once per input

**Severity:** low
**Status:** fixed
**Found by:** @otaliptus (L-8)

`TxGraph::insert` called `cluster_of` (a full walk and linearize) once
per mempool input to find the cluster representative, and the pre-insert
limit check did the same once per parent.

Insert now walks each touched component once, without linearizing, to
drop the old worst-chunk index. The limit check is one membership walk.
`cluster_of` still runs once per insert to publish the new worst chunk.

**Regression:** `rbitcoin-mempool`
`accept::tests::two_parent_insert_builds_the_cluster_once`.
