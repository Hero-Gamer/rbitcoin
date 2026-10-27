Fixed

- `estimatesmartfee` and `estimaterawfee` read `conf_target` as Core's 32-bit integer. `4294967297` was cut to 32 bits and answered as target 1; a value outside the int range, or a fraction, is now `-1` `JSON integer out of range`, and `-1` is `-8` like `0` and `1009`.
