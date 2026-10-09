Fixed

- **`invalidateblock` of the tip honors `preciousblock` on an equal-work fork.**
  The same tie rule as an ordinary reorg applies: more total work still wins,
  then the precious branch, then the earlier held tip.
