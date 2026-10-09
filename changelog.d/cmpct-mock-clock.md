Fixed

- **Compact fuzz shares Core's mock clock.** Header stamps stay within
  two hours of regtest genesis, and the follow accept uses that same
  clock. A split panic includes Core's `submitblock` reason.
