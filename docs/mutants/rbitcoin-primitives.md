# rbitcoin-primitives Mutants - Full Specifics Index

Generated from 16m run: 4 missed + 8 timeouts = 12 entries

| File | Total | Missed | Timeout |
| :--- | ---: | ---: | ---: |
| `script_sigops.rs` | 6 | 2 | 4 |
| `hex.rs` | 2 | 0 | 2 |
| `lib.rs` | 2 | 2 | 0 |
| `scriptnum.rs` | 2 | 0 | 2 |

## script_sigops.rs (6)

### line 13
- [ ] TIMEOUT - `replace += with *= in script_sigop_count`

### line 14
- [ ] TIMEOUT - `replace <= with > in script_sigop_count`

### line 41
- [ ] `replace > with < in script_sigop_count`
- [ ] `replace > with == in script_sigop_count`
- [ ] TIMEOUT - `replace > with >= in script_sigop_count`

### line 44
- [ ] TIMEOUT - `replace += with -= in script_sigop_count`

## hex.rs (2)

### line 97
- [ ] TIMEOUT - `replace += with *= in decode`

### line 103
- [ ] TIMEOUT - `replace from_digit -> Result<u8, HexError> with Ok(0)`

## lib.rs (2)

### line 62
- [ ] `replace > with >= in rbitcoin_subversion`

### line 124
- [ ] `replace schema_file_openable -> bool with true`

## scriptnum.rs (2)

### line 31
- [ ] TIMEOUT - `replace > with >= in scriptnum_encode`

### line 32
- [ ] TIMEOUT - `replace & with | in scriptnum_encode`
