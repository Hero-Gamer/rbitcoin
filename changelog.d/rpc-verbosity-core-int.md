Fixed

- **`getblock` and `getrawtransaction` read verbosity as Core's signed `int`.**
  A value above 4294967295 wrapped, so `4294967296` returned hex and
  `4294967297` returned the object. A negative value was rejected instead of
  returning hex. As in Core, a value at most 0 returns hex, a value outside
  `int` or a fraction or exponent is `-1` `JSON integer out of range`, and a
  non-number is `-3`. Boolean and null defaults are unchanged.
