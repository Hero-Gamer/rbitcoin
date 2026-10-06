Fixed

- **Tip-follow picks the held branch with the most total chain work.**
  Held side branches were ranked by work from their own fork point. A
  later branch with more total work could lose a tie to an earlier
  branch and never be tried. A branch that fails connect no longer
  hides a lighter valid branch that still beats the tip.
- **A failed block keeps the heavier valid part of its branch.** When a
  block in a side branch fails connect, the node stays on the blocks
  before it if they have more work than the old tip, as Core does. The
  failure is no longer reported for the valid block that triggered the
  reorg, so `submitblock` accepts it. The failed block itself is still
  rejected.
- **A side block's header is checked before the tip moves.** A sibling
  of the tip that claimed more work with wrong `nBits` disconnected the
  tip before it was validated and was not restored. `submitblock` and
  compact blocks could rewind the tip this way, one block per sibling.
