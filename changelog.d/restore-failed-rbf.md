Fixed

- **A failed replacement keeps the transactions it conflicted with.** If the new transaction does not stay in the mempool, one-transaction submit and package submit put those conflicts back, including a conflict under the fee floor and a parent that only paid its fee together with its child.
