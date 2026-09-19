# 00 — SOCKS5 proxy for P2P outbound

## Goal

Operator sets `--proxy 127.0.0.1:9050` (and optional `--onion` SOCKS). Every
P2P outbound dial goes through SOCKS5 CONNECT. DNS for seeds is **remote**
(SOCKS domain CONNECT), not a local stub resolver. `--proxy-randomize` (default
on) uses a fresh SOCKS username per peer so Tor isolates circuits (Core
`-proxyrandomize`). Plan 06 reuses the same isolation seam for one-shot
broadcast dials.

## Constraints

- System Tor already running; we are a SOCKS **client** only. No Arti.
- Hand-roll SOCKS5 in `rbitcoin-net` unless a spike shows a deny-clean crate is
  smaller. No `tokio-socks` by default.
- Fake SOCKS listener in unit tests. No live Tor in CI.
- Today `TcpStream::connect` is in
  [`ibd/peer_io.rs`](../../crates/rbitcoin-net/src/ibd/peer_io.rs)
  `spawn_peer`, [`service.rs`](../../crates/rbitcoin-net/src/service.rs)
  `prepare_outbound_session` (and a second outbound path ~line 556). One
  `dial` helper must own all of those.
- Seeds must not `ToSocketAddrs` locally when a proxy is set
  ([`seeds.rs`](../../crates/rbitcoin-net/src/seeds.rs) `resolve_*`).

## Out of scope

Onion `NetAddr` / `.onion` hostnames (02). Tor control / `ADD_ONION` (03).
I2P SAM (04). Ephemeral broadcast policy (06) — only the dial seam.

## Steps

### Step 1 — SOCKS5 CONNECT IPv4 against a fake server

- **Contract:** `socks5_connect(proxy, target_ipv4, creds)` writes a valid
  SOCKS5 greeting + CONNECT and returns the duplex stream; fake server sees
  ATYP=1 and the four address bytes.
- **Red:** `cargo test -p rbitcoin-net socks5_connect_ipv4_against_fake_proxy`
  — bind a local TCP fake SOCKS5, assert method and CONNECT bytes, echo a
  byte through the tunnel.
- **Green:** `rbitcoin-net` module `socks.rs` (or `proxy.rs`): greeting
  NOAUTH or USERNAME/PASSWORD, CONNECT, reply 0x00.
- **Refactor:** keep the fake proxy as a test helper, not production.
- **Verify:** `cargo test -p rbitcoin-net socks5_`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 2 — Domain CONNECT (remote DNS)

- **Contract:** CONNECT with ATYP=3 and the hostname bytes; we do **not** call
  `ToSocketAddrs` on the target when using domain form.
- **Red:** `socks5_connect_domain_does_not_resolve_locally` — fake proxy records
  ATYP=3 + `seed.example:8333`; fail the test if the helper resolved to an IP
  first.
- **Green:** domain form on the same helper.
- **Refactor:** one CONNECT encoder for IPv4 / IPv6 / domain.
- **Verify:** `cargo test -p rbitcoin-net socks5_`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 3 — Isolation credentials

- **Contract:** `ProxyCreds::fresh()` is a random username (and password);
  SOCKS USERNAME/PASSWORD subnegotiation uses those bytes. Same creds reused
  only when the caller passes them (long-lived peer). A one-shot API
  (`dial_isolated`) always calls `fresh()`.
- **Red:** `socks5_username_password_seen_by_fake_proxy` plus
  `dial_isolated_uses_new_creds_each_call`.
- **Green:** creds type + `dial_isolated` on the helper. Plan 06 will call
  `dial_isolated`; do not implement broadcast policy here.
- **Refactor:** default long-lived dials also `fresh()` when
  `proxy_randomize` is on (wired in step 6).
- **Verify:** `cargo test -p rbitcoin-net socks5_`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 4 — All P2P outbound uses `dial`

- **Contract:** with a process proxy set, `spawn_peer` and
  `prepare_outbound_session` never `TcpStream::connect` the peer directly;
  they CONNECT through the proxy. Without a proxy, behavior is unchanged
  (direct connect).
- **Red:** `cargo test -p rbitcoin-net outbound_dial_uses_proxy_when_set` —
  fake SOCKS + a fake v2 listener behind it; one outbound handshake
  succeeds only via the proxy. Direct-connect regression: proxy unset still
  reaches a local listener.
- **Green:** thread a `Dialer` (direct | socks) into IBD spawn and tip-follow
  prepare. `P2PNode` / IBD config holds the proxy socket.
- **Refactor:** delete duplicated `TcpStream::connect` at the two
  `service.rs` sites and `peer_io.rs`.
- **Verify:** `cargo test -p rbitcoin-net outbound_dial_` plus existing
  `node_run_p2p_short` still passes (no proxy).
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 5 — `--proxy` / `--onion` CLI and conf

- **Contract:** `--proxy HOST:PORT` and conf `proxy=` set the SOCKS endpoint.
  `--onion HOST:PORT` is the SOCKS used for onion (02); until 02, storing the
  socket is enough. Empty/invalid address is a start error. Default: no proxy.
- **Red:** `cargo test -p rbitcoin-node proxy_conf_and_cli` — parse
  `proxy=127.0.0.1:9050`; reject `proxy=`; `--onion` independent field.
- **Green:** [`ListenOpts`](../../crates/rbitcoin-node/src/config.rs) fields;
  `apply_kv`; kebab CLI. Pass into `run_p2p` / `P2PNode`.
- **Refactor:** none unless flags duplicate.
- **Verify:** `cargo test -p rbitcoin-node proxy_` plus `rbitcoin-node --help`
  lists kebab `--proxy` / `--onion`.
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 6 — `--proxy-randomize` default on; seeds via SOCKS domain

- **Contract:** `--proxy-randomize` defaults **on**. When `--proxy` is set,
  DNS seed hostnames are SOCKS domain CONNECTs (or skipped in favor of
  `--connect` / `--seed-node` strings passed as domain). Local
  `resolve_dns_seeds` is not used on the proxy path.
- **Red:** `proxy_randomize_defaults_on`; `dns_seeds_not_resolved_locally_when_proxy`.
- **Green:** flag + seed path. `--proxy-randomize=0` reuses one credential
  (Core off).
- **Refactor:** seed resolution and dial share the domain CONNECT encoder.
- **Verify:** `cargo test -p rbitcoin-node proxy_randomize_` ;
  `cargo test -p rbitcoin-net dns_seeds_`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 7 — OPERATOR row

- **Contract:** [`OPERATOR.md`](../../OPERATOR.md) CLI table documents
  `--proxy`, `--onion`, `--proxy-randomize` (default on). No COMPAT change.
- **Red:** none (docs).
- **Green:** OPERATOR table + short “P2P via system Tor SOCKS” paragraph.
- **Refactor:** n/a
- **Verify:** grep the flag names in OPERATOR.md.
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

## Test budget

Unit fake-proxy tests in `rbitcoin-net`. One slim outbound handshake through
the fake proxy. Do not add a live-Tor scenario.

## Risks / follow-ups

IPv6 ATYP=4 needed for `--onlynet=ipv6` through Tor (still this PR if cheap;
else a step in 02). Onion hostnames need 02. Plan 06 must not fork a second
SOCKS client.
