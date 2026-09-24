# 02 — Onion AddrMan, `--only-net`, SOCKS dial of `.onion`

## Goal

The peer book stores BIP155 Tor v3 addresses, persists them, and dials them
through the SOCKS client from [00](./00-socks-proxy.md). `--only-net=onion`
(repeatable with ipv4/ipv6) filters the dial set. `--connect` /
`--seed-node` accept `….onion:port`.

## Constraints

- Depends on **00** (SOCKS domain CONNECT + `--onion` SOCKS endpoint).
- [`learn_addrv2`](../../crates/rbitcoin-net/src/peers.rs) only
  `add_learned` when `a.socket_addr()` succeeds — onion/I2P are dropped.
- [`AddrMan`](../../crates/rbitcoin-net/src/seeds.rs) is `SocketAddr`-keyed;
  peers file magic `rbitcoin-peers-v1` parses `SocketAddr`.
- `--connect` is `Vec<SocketAddr>` in `ListenOpts`. `--seed-node` is already
  `Vec<String>`.
- Leave `NetAddr` extension points for `I2p` ([04](./04-i2p.md)) and `Cjdns`
  ([08](./08-cjdns.md)); do not implement those dials here.
- Onion dial **requires** SOCKS (`--proxy` or `--onion`). Fail start if
  `--only-net=onion` and neither is set.

## Out of scope

I2P, CJDNS, Tor control, `--listen-onion`, wallet onions.

## Steps

### Step 1 — `NetAddr` and onion parse

- **Contract:** `NetAddr::Ip(SocketAddr)` and
  `NetAddr::Onion { pk: [u8; 32], port }`. Display is the `.onion` hostname
  + port. Parse `wwwwwwwww….onion:8333` (56-char v3). Invalid checksum /
  length is error. Equality/hash stable for AddrMan keys.
- **Red:** `cargo test -p rbitcoin-net netaddr_onion_parse_roundtrip` ;
  reject truncated and bad checksum.
- **Green:** type in `rbitcoin-net` (new `netaddr.rs` or `seeds.rs`). Use
  `bitcoin::p2p::address::AddrV2::TorV3` if it already owns encode/decode.
- **Refactor:** no `String` keys in AddrMan.
- **Verify:** `cargo test -p rbitcoin-net netaddr_`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 2 — AddrMan + `learn_addrv2` keep Tor v3

- **Contract:** `learn_addrv2` inserts `NetAddr::Onion` for `AddrV2::TorV3`.
  IPv4/IPv6 still insert `Ip`. I2P/CJDNS still ignored (04/08).
- **Red:** `overlay_config` (`learn_addrv2_keeps_tor_v3`) — feed one TorV3 row; `entries()`
  contains the onion. Existing IPv4 learn tests still pass.
- **Green:** rekey AddrMan to `NetAddr`. Update `take_dial_candidates` to
  return `NetAddr`.
- **Refactor:** `SocketAddr`-only helpers become `Ip` arms.
- **Verify:** `cargo test -p rbitcoin-net learn_addrv2` ;
  `cargo test -p rbitcoin-net addrman`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 3 — peers file v2

- **Contract:** persist onion as a `.onion:port` token. Old `rbitcoin-peers-v1`
  IPv4/IPv6 files still load. New writes use `rbitcoin-peers-v2` **or** v1
  plus onion tokens without wiping (pick one; no silent truncate of onion
  lines). Missing file still empty book.
- **Red:** `peers_file_roundtrip_onion`; `peers_file_v1_ipv4_still_loads`.
- **Green:** [`AddrMan::load` / persist](../../crates/rbitcoin-net/src/seeds.rs).
  Same-commit format note in OPERATOR (peers file), not SCHEMA.md (not
  Class A/B/C).
- **Refactor:** one line parser for IP and onion.
- **Verify:** `cargo test -p rbitcoin-net peers_file_`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 4 — SOCKS dial of onion

- **Contract:** `dial(NetAddr::Onion)` CONNECTs the `.onion` hostname through
  `--onion` SOCKS if set, else `--proxy`. Fake SOCKS records ATYP=3 and the
  hostname. Direct `TcpStream::connect` is not used for onion.
- **Red:** `dial_onion_uses_socks_domain_connect`.
- **Green:** Dialer onion arm. IBD `spawn_peer` and tip-follow prepare take
  `NetAddr` (or resolve onion before handshake local-addr logging).
- **Refactor:** handshake `addr: SocketAddr` peer identity: use a display
  string / `NetAddr` in logs; do not invent a fake IPv4.
- **Verify:** `cargo test -p rbitcoin-net dial_onion_`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 5 — `--connect` / `--seed-node` parse `.onion`

- **Contract:** `--connect foo.onion:8333` and `seed_node=` store onion
  `NetAddr`. Invalid onion is a start error. Existing `IP:port` still works.
- **Red:** `cargo test -p rbitcoin-node overlay_config` (`connect_onion_and_ipv4`).
- **Green:** `ListenOpts.connect: Vec<NetAddr>` (or parallel onion list —
  prefer one Vec).
- **Refactor:** delete `Vec<SocketAddr>`-only connect if fully replaced.
- **Verify:** `cargo test -p rbitcoin-node connect_onion`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 6 — `--only-net` and start fail without SOCKS

- **Contract:** `--only-net=ipv4|ipv6|onion` repeatable. Dial/learn skip other
  networks. `--only-net=onion` without `--proxy` and without `--onion` is
  `NodeError::Config` naming SOCKS. `--only-net=i2p` / `cjdns` unknown until
  04/08 (error: unknown network).
- **Red:** `only_net_onion_filters_ipv4_candidates`;
  `overlay_config` (`only_net_onion_without_proxy_is_config_error`).
- **Green:** `apply_kv` `only_net=`; filter in AddrMan take + DNS/fixed seed
  inject.
- **Refactor:** single `OnlyNet` set on listen opts.
- **Verify:** `cargo test -p rbitcoin-node only_net_` ;
  `cargo test -p rbitcoin-net only_net_`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 7 — OPERATOR + NixOS module

- **Contract:** document `--only-net`, onion `--connect`, peers v2, SOCKS
  required for onion. Module: `services.rbitcoin.onlyNet` list; eval asserts
  `--only-net onion` when set. SOCKS still comes from 00’s `proxy`.
- **Red:** eval assert for `--only-net`.
- **Green:** module + eval + OPERATOR P2P section.
- **Verify:** `nix build .#checks.x86_64-linux.nixos-module-eval --no-link`;
  grep OPERATOR.
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

## Test budget

Parse/persist/filter units + one fake-SOCKS onion CONNECT. No live Tor.

## Risks / follow-ups

Handshake code wants `SocketAddr` for `local` bind and self-connect checks.
Onion outbound `local_addr()` is the SOCKS proxy IP — do not treat that as
the peer. I2P/CJDNS variants must not require a second AddrMan.
