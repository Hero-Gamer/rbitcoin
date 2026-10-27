Fixed

- `estimatesmartfee` and `estimaterawfee` read `conf_target` as Core's 32-bit integer. `4294967297` was cut to 32 bits and answered as target 1; a value outside the int range, or a fraction, is now `-1` `JSON integer out of range`, and `-1` is `-8` like `0` and `1009`.
- When live arrivals priced above 1000 sat/vB overfilled the next blocks at every rate the flow estimator prices, the 1- to 5-block estimates fell back to the frontier, or to the 0.1 sat/vB minimum relay rate when the pool held less than those blocks, and capped every longer target. They now quote the 1000 sat/vB top of that range.
