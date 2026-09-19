# 08 — CJDNS

## Goal

Reach and be reached by BIP155 CJDNS peers on the **kernel IPv6 overlay**
(`fc00::/8`) without Arti, SOCKS, or SAM. Core-shaped `--cjdnsreachable`:
without it, CJDNS addrs are unroutable (do not dial; do not treat `fc00::/8`
as advertisable). `--onlynet=cjdns`. Inbound is `--listen` on the host’s
cjdns IPv6 — no ISP TCP forward; the overlay delivers.

## Constraints

- Depends on **02** (`NetAddr` + `--onlynet` + peers v2). Independent of
  Tor/I2P.
- No live cjdns router in CI. Do not auto-detect a TUN.
- Dial: `TcpStream::connect` to that IPv6 (OS routing).
- [`ip_is_advertisable`](../../crates/rbitcoin-net/src/peers.rs) today:
  IPv6 only rejects unspecified/loopback — ULA `fc00::/8` may already look
  advertisable. Pin: **off** unless `--cjdnsreachable`.
- If `--listen=0`, cjdns inbound is off unless an explicit
  `--listen <cjdns-ip>:port`.

## Out of scope

Tor, I2P, UPnP, embedding a cjdns daemon.

## Steps

### Step 1 — `NetAddr::Cjdns` + `learn_addrv2`

- **Contract:** `NetAddr::Cjdns { ip: Ipv6Addr, port }` from
  `AddrV2::Cjdns`. Reject non-`fc00::/8`. Round-trip persist in peers v2.
- **Red:** `netaddr_cjdns_addrv2_roundtrip`; `learn_addrv2_keeps_cjdns`;
  `cjdns_rejects_global_unicast`.
- **Green:** extend `NetAddr`; `learn_addrv2` arm.
- **Refactor:** do not store CJDNS as a bare `SocketAddr` without a tag if
  `--onlynet` cannot distinguish it from IPv6.
- **Verify:** `cargo test -p rbitcoin-net netaddr_cjdns` ;
  `cargo test -p rbitcoin-net learn_addrv2`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 2 — `--cjdnsreachable` gates dial and advertise

- **Contract:** flag default **off**. Off: do not dial CJDNS `NetAddr`;
  `ip_is_advertisable` / announce skip `fc00::/8`. On: dial via
  `TcpStream::connect`; `fc00::/8` may appear in localaddresses if we
  `--listen` on one.
- **Red:** `cjdns_not_dialed_when_unreachable`;
  `fc00_not_advertisable_without_cjdnsreachable`.
- **Green:** hub/listen flag; Dialer IPv6 vs Cjdns arm (direct connect
  both, filter before dial).
- **Refactor:** keep one TCP connect path for IP and CJDNS.
- **Verify:** `cargo test -p rbitcoin-net cjdns_reachable_`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 3 — `--onlynet=cjdns` CLI

- **Contract:** `--onlynet=cjdns` is valid. Implies we only dial CJDNS
  (need `--cjdnsreachable` or treat onlynet=cjdns as enabling reachable —
  **pin: onlynet=cjdns without cjdnsreachable is Config error**, same
  honesty as onion-without-SOCKS).
- **Red:** `onlynet_cjdns_without_reachable_is_config_error`;
  `onlynet_cjdns_filters_ipv4`.
- **Green:** `onlynet` parser from 02; node validate.
- **Verify:** `cargo test -p rbitcoin-node onlynet_cjdns`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 4 — Listen on a CJDNS address

- **Contract:** `--listen fcfd:…:port` binds that IPv6 when the OS has it.
  Tests: bind `::1` is **not** cjdns; unit-parse the listen addr as
  `NetAddr::Cjdns` vs `Ip`. Config accepts an `fc00::/8` listen string.
  `--listen=0` does not bind cjdns.
- **Red:** `listen_cjdns_addr_parses`; `listen_zero_does_not_bind_cjdns`
  (may already be 01).
- **Green:** `push_p2p_listen` already takes `SocketAddr` — V6 fc00 works
  if the stack allows. Document OS requirement. No TUN in CI: parse/bind
  tests use a skip if `bind` fails with addr-not-available, **or** only
  parse-level tests.
- **Refactor:** none.
- **Verify:** `cargo test -p rbitcoin-node listen_cjdns`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 5 — OPERATOR + NixOS module

- **Contract:** `--cjdnsreachable`, `--onlynet=cjdns`, listen on cjdns
  IPv6, no ISP forward, no daemon in-process, no CI router. Module:
  `cjdns.reachable`; P2P bind on the cjdns address when set;
  `After`/`Wants` `cjdns.service`. Eval asserts argv. Extend runtime test;
  label **`nixos-module-runtime`**. Do not start a cjdns TUN in CI.
- **Red:** eval assert for `--cjdnsreachable`.
- **Green:** module + eval + runtime test + OPERATOR.
- **Verify:** `nix build .#checks.x86_64-linux.nixos-module-eval --no-link`;
  grep cjdns. Poll `--interest nixos-module-runtime`.
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

## Test budget

Parse/learn/filter units. Optional bind skip. No cjdns TUN in CI.

## Risks / follow-ups

`fc00::/8` overlaps RFC4193 ULA. Core’s `-cjdnsreachable` is the same
compromise. Do not dial random ULA without the flag.
