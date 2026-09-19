# 07 — P2P onion inbound (`listenonion`)

## Goal

Accept Bitcoin P2P (BIP324) inbound over a Tor hidden service without any
ISP TCP forward and **without Arti**. Core `-listenonion`: even with
`--listen=0` / no clearnet bind, listen on **127.0.0.1**, `ADD_ONION`
virtual default P2P port → that loopback, persist `{datadir}/onion/p2p.priv`.
Gossip **the onion** via `addrv2` to outbound peers (that is how inbound
arrives). Do not gossip a home IPv4.

## Constraints

- Depends on **01** (outbound-only vs bind modes) and **03** (`ADD_ONION`
  helper). Does not wait on 04–06.
- Not Arti. System tor control port.
- Inbound sessions look like `127.0.0.1` (Tor maps to loopback). Ordinary
  BIP324 accept path.
- `max_inbound` applies. `--max-inbound 0` **refuses** `--listen-onion`
  (need at least one inbound slot).
- `--no-discover` still **allows onion announce** when listen-onion is on
  (Core: onion is not an interface scan). `--no-discover` still suppresses
  home IP announce (01).
- Fake control port + a local client connecting to the loopback P2P port
  (as if Tor forwarded). No live Tor in CI.

## Out of scope

Wallet onions (03/05). I2P incoming (04). CJDNS (08). Clearnet UPnP.

## Steps

### Step 1 — Loopback P2P bind with `--listen=0` + `--listen-onion`

- **Contract:** `--listen-onion` with `--no-listen` starts a `127.0.0.1:0` or
  `127.0.0.1:<default_p2p>` TcpListener and runs the inbound accept loop.
  No `0.0.0.0` bind. `--listen-onion` + `--max-inbound 0` is Config error.
- **Red:** `listen_onion_binds_loopback_when_nolisten`;
  `listen_onion_refused_when_max_inbound_zero`.
- **Green:** extend `P2PNode` start: optional onion-loopback listener
  distinct from clearnet `listen.p2p`. Reuse `spawn_inbound_accept`.
- **Refactor:** 01’s outbound-only start becomes “no *clearnet* listener”.
- **Verify:** `cargo test -p rbitcoin-net listen_onion_bind_` ;
  `cargo test -p rbitcoin-node listen_onion_`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 2 — `ADD_ONION` for P2P

- **Contract:** control helper maps virtual network default port (8333 /
  signet 38333 / …) to the loopback bind. Persist `{datadir}/onion/p2p.priv`.
  Same ServiceID after restart (fake control).
- **Red:** `p2p_add_onion_persists_key` (reuse 03 tests with a p2p path).
- **Green:** `run_p2p` after loopback bind.
- **Refactor:** `OnionServices.p2p` next to electrum/esplora.
- **Verify:** `cargo test -p rbitcoin-node p2p_add_onion`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 3 — Inbound BIP324 on that loopback

- **Contract:** a local client `TcpStream::connect` to the loopback P2P
  port completes v2 handshake and counts as inbound in `getpeerinfo`
  (`inbound: true`).
- **Red:** `listen_onion_loopback_inbound_handshake` (existing inbound
  tests + onion-mode start).
- **Green:** no special handshake; accept loop already does this.
- **Refactor:** n/a if step 1 reused accept.
- **Verify:** `cargo test -p rbitcoin-net listen_onion_loopback_inbound`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 4 — Self-announce onion, not home IP

- **Contract:** `take_self_announce_msg` / addrv2 self-announce sends
  `AddrV2::TorV3` for our P2P onion when listen-onion is on. Does not send
  `--external-ip` IPv4 unless discover is on **and** a clearnet listen
  exists. `getnetworkinfo.localaddresses` includes the onion hostname.
- **Red:** `self_announce_onion_not_external_ip`; extend
  `getnetworkinfo_localaddresses_from_externalip` for onion rows.
- **Green:** hub stores onion `NetAddr`; announce path in
  [`peers.rs`](../../crates/rbitcoin-net/src/peers.rs)
  `take_self_announce_msg` (today IP-only).
- **Refactor:** `advertise_local_socket` stays IP; add
  `advertise_onion()` rather than stuffing onion into `SocketAddr`.
- **Verify:** `cargo test -p rbitcoin-net self_announce_onion` ;
  `cargo test -p rbitcoin-rpc getnetworkinfo_localaddresses`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 5 — OPERATOR

- **Contract:** `--listen-onion`, interaction with `--no-listen` and
  `--max-inbound`, onion in `getnetworkinfo`, no Arti.
- **Red:** none.
- **Green:** OPERATOR P2P.
- **Verify:** grep `--listen-onion`.
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

## Test budget

Fake control + loopback inbound handshake. No live Tor.

## Risks / follow-ups

Tor source IP is 127.0.0.1: inbound eviction / netgroup protect may treat
all onion inbounds as one group (Core has the same shape). Do not “fix”
that in this PR unless a test shows a DoS hole we already own. Feelers to
our own onion: self-connect refuse already exists — pin it still fires.
