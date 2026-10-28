Changed

- **`maxfeerate` is Core BTC/kvB.** `sendrawtransaction`,
  `testmempoolaccept`, and `submitpackage` read it as a BTC/kvB number or
  string, like Core. LND sends `0.1` there and could not publish a funding
  or cooperative-close transaction (`-8 maxfeerate must be an integer
  sat/vB`). The default is still 0.10 BTC/kvB (10,000 sat/vB) and `0` is
  still unlimited. `1` BTC/kvB or more is `-8`, so a caller still sending
  a whole sat/vB number other than `0` gets `-8`, not a different cap.
