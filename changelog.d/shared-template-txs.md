Changed

- `getblocktemplate` and the SV2 Template Provider select transactions as
  the mempool's own shared bodies instead of copying every selected
  transaction on each call.
