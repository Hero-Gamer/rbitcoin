Changed

- **Clock is single NodeClock source.** `NodeClock::new()` -> `Arc` is the only clock, `PeerHub::new(clock)` holds `pub clock`. Second mock battery `mock_now` / `set_mock_now` removed from `PeerHub`, `on_clock_jump()` is explicit. `setmocktime` writes `ChainHub` once, fanout removed.
