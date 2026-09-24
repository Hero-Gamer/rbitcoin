# 040 — Corrupt store lengths

**Severity:** medium
**Status:** fixed
**Found by:** coordinated review, 2026-09-21 and 2026-09-22 (C15, Third #05, #17B, #18, L-4)

A uleb128 byte at shift 63 whose payload is wider than one bit is
`PackError("uleb128 overflow")`. A final byte of 0 or 1 at that shift
still decodes, including the encoding `write_uleb128` emits for
`u64::MAX`.

A seqsigwit CompactSize for a script, a witness count, or a witness item
that does not fit in the remaining bytes is `StoreError::Corrupt`. The
decoder does not add that length into the buffer index, and it does not
size a witness vector from the raw count. The same fit check covers the
v17 raw-script payload.

A BDZ file with `n > 1` and a zero modulus or zero vertex count is
`StoreError::Corrupt("bdz mphf: zero modulus")` at read. Indexing does
not divide by that modulus.

Fuse8 already rejects `segment_length == 0` before any division
(`decode_body_rejects_invalid_geometry_and_truncation`). Header tx-list
expansion rejects a count above `u32::MAX` before `Vec::with_capacity`;
that capacity does not overflow `isize` on 64-bit. Those two are clean.

`map_prefix` already refuses a length past the file
(`map_prefix_longer_than_file_is_corrupt`). Truncating a sealed map
after it is mapped is fatal external corruption.

**Regression:** `rbitcoin-primitives` `compact::tests::compact_and_uleb_error_paths`,
`rbitcoin-store` `tx_table::tests::packed_encode_decode_flags_and_error_arms`,
`bdz::tests::read_packed_zero_modulus_or_vertices_is_corrupt`.
