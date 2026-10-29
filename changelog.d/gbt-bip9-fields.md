Fixed

- **`getblocktemplate` carries the BIP9 and BIP145 fields.** The template
  now has `vbavailable` and `vbrequired`, so typed clients such as Rust
  `bitcoincore-rpc` can parse it. `rules` is Core's list (`csv`, `!segwit`,
  `taproot`, `!signet` on signet), and `coinbaseaux` is `{}`. `version`
  follows Core's regtest `testdummy` deployment (bit 28 while it is started
  or locked in). Other networks get `0x20000000`, not `0x30000000`. Before
  segwit the template uses the legacy sigop and size limits and omits
  `weightlimit`. Signet requires the `signet` rule and returns
  `signet_challenge`. Request errors match Core: a proposal needs no rules,
  a missing `data` is `-3`, a non-string rule is `-3`, and a mode that is not
  `template` or `proposal` is `-8` `Invalid mode`. Esplora
  `GET /block-template` and the SV2 template version follow.
