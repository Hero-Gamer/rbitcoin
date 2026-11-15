Security

- **Automatic outbound peers take one netgroup each.** The refill, the
  stale-tip replacement, and IBD catch-up dials now open at most one
  automatic outbound connection per IPv4 `/16`, IPv6 `/32`, or asmap ASN,
  as Core does, and leave a slot empty rather than reuse a group. Before,
  a book whose dialable rows sat in two `/16`s filled all 16 outbound slots
  from them. Inbound peers no longer hold a group, so they cannot steer
  outbound picks. Manual (`--connect`, `addnode`), Tor, I2P, CJDNS, and
  loopback or other unroutable rows are exempt; an `addnode` peer made
  during IBD still holds its group. With `--asmap`, an address the map
  does not cover keeps its prefix group instead of sharing one group with
  every other unmapped address. When every IP row the rule allows was
  tried in the last ten minutes, a refill pass redials only the one tried
  longest ago, not all of them, so reconnecting after an outage starts
  one peer per pass, and that pass adds at most one Tor or I2P row.
- **Netgroups follow Core's `GetGroup`.** An IPv6 address that carries an
  IPv4 (IPv4-mapped, NAT64, SIIT, 6to4, Teredo) is grouped, and looked up
  in the asmap, as that IPv4, so a 6to4 address no longer opens a second
  group beside its IPv4 `/16`. Hurricane Electric `2001:470::/32` is
  grouped by `/36`, and CJDNS by the four bits after `fc`. Inbound
  eviction uses the same groups. The ten-minute pause after a slot
  eviction does too, except that it keys a CJDNS peer on its `/32`.
