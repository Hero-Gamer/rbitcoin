Changed

- Nightly mutants skip `rbitcoin-bench`. It is an optional host client, not a test gate. `#[mutants::skip]` marks an expression that cargo-mutants will not report as missed.
