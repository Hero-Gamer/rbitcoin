Fixed

- **`getnetworkinfo`, `getblockchaininfo`, and `getmininginfo` answer
  quickly again.** Their `warnings` check read every header since genesis
  once per version bit on each call (about 4.7 s at mainnet tip). It now
  keeps its place and reads each completed 2016-block period once.
