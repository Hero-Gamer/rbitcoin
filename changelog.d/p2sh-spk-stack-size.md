Fixed

- **Consensus: a P2SH spend whose scriptSig leaves 1000 stack items is
  invalid.** Core runs the P2SH scriptPubKey `HASH160 <20> EQUAL` on the
  scriptSig stack, and its 20-byte push goes past the 1000-item stack
  limit. We only compared the hash, so we accepted a spend that Core
  rejects.
