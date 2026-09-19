# 01 — Listen off, inbound zero, no discover

## Goal

A home node can run **outbound-only** P2P: no clearnet (or even LAN) P2P
listener, `--max-inbound 0`, no self-announce of a home IP. This is the
posture that does not wait for ISP port forwards. Plan 07 later adds a
**loopback-only** P2P bind for onion inbound; this plan must leave that
possible (outbound-only is a start mode, not “never accept”).

## Constraints

- Today [`config.rs`](../../crates/rbitcoin-node/src/config.rs) `validate`
  rejects `max_inbound == 0` (`must be >= 1`).
- [`P2PNode::start_with_agent`](../../crates/rbitcoin-net/src/service.rs)
  always `TcpListener::bind` and `max_inbound.max(1)`.
- [`run.rs`](../../crates/rbitcoin-node/src/run.rs) defaults listen to
  `127.0.0.1:<p2p>` when `listen.p2p` is `None`.
- [`take_self_announce_msg`](../../crates/rbitcoin-net/src/peers.rs) already
  requires `advertise_local_socket()` which needs `--external-ip` and a
  listen port. Still add `--no-discover` so we never start scanning or
  filling `external_ips` later.
- `--max-inbound 0` means **no inbound slots**. Plan 07 requires
  `max_inbound >= 1` when `--listen-onion` is on (clearnet still unbound).

## Out of scope

Onion AddrMan (02). Tor control (03). Loopback onion bind (07). CJDNS bind
(08). SOCKS (00) may already be merged; this PR does not require it.

## Steps

### Step 1 — `max_inbound=0` is valid config

- **Contract:** `--max-inbound 0` and conf `max_inbound=0` start; validate no
  longer errors `must be >= 1`. `max_outbound` stays `>= 1`.
- **Red:** `cargo test -p rbitcoin-node max_inbound_zero_is_allowed` — today
  fails on validate. Also pin `max_outbound=0` still errors.
- **Green:** drop the inbound `>= 1` check; keep explicit flag tracking.
- **Refactor:** comments / help text that say 125 default.
- **Verify:** `cargo test -p rbitcoin-node max_inbound_`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 2 — `--listen=0` / `--no-listen` does not default-bind

- **Contract:** `--listen=0` or `--no-listen` (and conf `listen=0`) leaves
  `listen.p2p = None` and **does not** bind in `run_p2p`. `--listen 127.0.0.1:18444`
  still binds. Repeatable `--listen` extra binds unchanged.
- **Red:** `listen_zero_does_not_default_loopback`; CLI `--no-listen`; conf
  `listen=0`. Help lists kebab `--no-listen`.
- **Green:** parse `0` / `false` / `--no-listen` as “no P2P bind”. Change
  `run_p2p` unwrap_or default. First `--listen ADDR` still clears extras
  ([`cli.rs`](../../crates/rbitcoin-node/src/cli.rs) `saw_listen`).
- **Refactor:** one enum or `Option` — do not add a parallel bool that
  fights `listen.p2p`.
- **Verify:** `cargo test -p rbitcoin-node listen_zero_`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 3 — `P2PNode` outbound-only start

- **Contract:** `P2PNode` can start with no `TcpListener`. Outbound dials
  still work (`addnode` / IBD `/` tip-follow). `addconnection inbound`
  refuses. `local_addr` is not a bound P2P port (document:
  unspecified or `127.0.0.1:0` unused — pick one and pin).
- **Red:** `cargo test -p rbitcoin-net p2p_outbound_only_dials_without_listener`
  — two nodes: seeder listens; follower `start_outbound_only` + connect;
  handshake. Inbound `addconnection` still errors.
- **Green:** `start_outbound_only` (or `start_with_agent` with
  `listen: Option<SocketAddr>`). Accept loop skipped. `max_inbound == 0`
  also skips accept even if a leftover listener exists.
- **Refactor:** do not duplicate the dial task; share with `start_with_agent`.
- **Verify:** `cargo test -p rbitcoin-net p2p_outbound_only_` ;
  existing `node_run_p2p_short` still binds.
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 4 — `--no-discover` and no self-announce

- **Contract:** `--no-discover` (default **off** = today’s behavior). When
  on, `take_self_announce_msg` is None even if `--external-ip` is set.
  `getnetworkinfo.localaddresses` is empty unless `--external-ip` **and**
  discover is on **and** a listen port is bound. With `--listen=0` and no
  onion (07), localaddresses is empty.
- **Red:** `no_discover_suppresses_self_announce` in `rbitcoin-net` (extend
  `externalip_is_advertised_once_then_after_a_day` /
  `loopback must not be advertised`). Node: `no_discover_conf`.
- **Green:** flag on hub; `take_local_addr_due` respects it.
- **Refactor:** keep `ip_is_advertisable` as-is (loopback already filtered).
- **Verify:** `cargo test -p rbitcoin-net externalip_` ;
  `cargo test -p rbitcoin-rpc getnetworkinfo_localaddresses`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 5 — OPERATOR

- **Contract:** OPERATOR CLI table: `--no-listen` / `--listen=0`,
  `--max-inbound 0`, `--no-discover`. Note that 07 will add loopback onion
  bind without clearnet listen.
- **Red:** none (docs).
- **Green:** OPERATOR.
- **Verify:** grep flags.
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

## Test budget

Config unit tests + one live two-node outbound-only handshake. Do not add a
full IBD scenario.

## Risks / follow-ups

`P2PNode.local_addr` is used by tests as the bind address. Outbound-only
tests must use the seeder’s addr, not the follower’s. Plan 07 must be able
to bind `127.0.0.1` without reopening “always listen”.
