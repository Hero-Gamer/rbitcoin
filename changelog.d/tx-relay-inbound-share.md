Changed

- **Block-relay-only peers get their own inbound slots (Core #28463).** The
  default `--max-inbound` rises from 125 to 200, and peers that relay
  transactions may hold at most half of the inbound slots. A new
  tx-relaying inbound past that half evicts another tx-relaying inbound
  under the usual eviction protection. If every one is protected, the new
  peer is closed. Block-relay-only inbounds are never evicted to make that
  room.
- **A peer that sent `relay=false` no longer gets mempool INVs.** The
  periodic and forced tx announcements went to it anyway; only our own
  unbroadcast transactions are announced to it now, as on the immediate
  announce path.
