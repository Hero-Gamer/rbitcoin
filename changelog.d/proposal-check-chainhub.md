Changed

- **`getblocktemplate` proposal mode prices the coinbase.** A proposal whose
  coinbase pays more than the block subsidy plus fees is rejected as
  `bad-cb-amount`, after the structure checks, as Bitcoin Core's
  `TestBlockValidity` does. The check now lives on `ChainHub`
  (`check_block_proposal`, returning the fee total) so other front ends
  such as the SV2 template provider run the same code as the RPC.
