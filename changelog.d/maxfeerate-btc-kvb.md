Changed

- **`maxfeerate` is Core BTC/kvB.** `sendrawtransaction`,
  `testmempoolaccept`, and `submitpackage` read it as a BTC/kvB number or
  string, like Core. LND sends `0.1` there and could not publish a funding
  or cooperative-close transaction (`-8 maxfeerate must be an integer
  sat/vB`). The default is still 0.10 BTC/kvB (10,000 sat/vB) and `0` is
  still unlimited. `1` BTC/kvB or more is `-8`, so a caller still sending
  a whole sat/vB number other than `0` gets `-8`, not a different cap.
- **`maxfeerate` and `maxburnamount` parse like Core.** Exponent forms such
  as `1e-05`, which Python and Go clients write for small values, are read.
  A bad or negative amount is `-3`, not `-8`, and leading spaces, leading
  zeros, `.5`, and `1.` are refused.
- **`submitpackage` applies `maxfeerate` like Core.** Each member's fee
  includes its `prioritisetransaction` delta and is measured over its
  sigop-adjusted size, as `testmempoolaccept` already did.
