Fixed

- **New block times respect the difficulty-period floors.** As in Core's
  `GetMinimumTime` (bitcoin/bitcoin#35949), a period's last block is not
  earlier than the period's first block, and a period's first block is at
  most 600 seconds before its parent. Both floors apply on every network.
  `getblocktemplate` `mintime`, the SV2 template time, and regtest
  `generate` follow them, so a node whose clock is behind still builds a
  valid block. `curtime` is now Core's: the later of the clock and
  `mintime`, not the tip's time plus one. The Esplora `GET /block-template`
  JSON follows.
