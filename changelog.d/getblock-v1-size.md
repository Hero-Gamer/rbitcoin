Fixed

- **`getblock` verbosity 1 carries `size`, `strippedsize`, and `weight`**,
  as Bitcoin Core does. They come from `txstat` (no block reconstruct) and are
  left out only when the body is unavailable. Stock mempool's block indexer
  stores `size` NOT NULL, so without them every block failed to save
  (`Column 'size' cannot be null`) and its tip, fees, and block list stalled.
