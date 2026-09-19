# 03 — Tor control `ADD_ONION` and Electrum hidden service

## Goal

Talk to **system tor** on the control port (cookie or password). Expose a
tested `ADD_ONION` helper that persists a private key. First product user:
map an Electrum virtual port to loopback `--electrum-listen` (plain TCP).
Log the `….onion:port`. Set Electrum `server.features.hosts` for that onion
(`tcp_port`, no `ssl_port`). RPC unix socket stays off the onion.

Plans [05](./05-wallet-onion.md) (Esplora) and [07](./07-p2p-onion-inbound.md)
(P2P listenonion) reuse this helper.

## Constraints

- Not Arti. Control protocol over TCP to `--tor-control` (default try
  `127.0.0.1:9051`).
- Cookie file or password. Typical cookie:
  `/run/tor/control.authcookie` or `--tor-control-cookie PATH`.
- Fake control port in tests. No live Tor in CI.
- Live in `rbitcoin-node` (operator), not a new crate. Net crate does not
  need to speak control.
- `{datadir}/onion/electrum.priv` so the hostname is stable across restarts.
- Electrum stays plain TCP ([`run_electrum`](../../crates/rbitcoin-electrum/src/server.rs)).
  No rustls.

## Out of scope

Esplora onion (05). P2P listenonion (07). TLS. SOCKS (00) is independent
(outbound); HS inbound does not use SOCKS.

## Steps

### Step 1 — Control AUTH against a fake port

- **Contract:** connect, `AUTHENTICATE` with hex cookie or quoted password,
  then `GETINFO version` succeeds. Wrong cookie → error, no panic. Protocol
  is `250` / `515` line-oriented (Core-compatible enough for AUTH + ADD_ONION).
- **Red:** `cargo test -p rbitcoin-node tor_control_auth_cookie_and_password`
  — fake control server; cookie file on disk; password path; reject bad
  cookie.
- **Green:** `tor_control.rs` in `rbitcoin-node`: connect, read protocol
  info if needed, AUTH, one command/response helper.
- **Refactor:** keep line parser small; no full Tor control library.
- **Verify:** `cargo test -p rbitcoin-node tor_control_`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 2 — `ADD_ONION` + persist key

- **Contract:** `ADD_ONION` with `NEW:ED25519-V3` or `ED25519-V3:<key>` and
  `Port=VIRT,TARGET`. Reply `ServiceID=` + optional `PrivateKey=` is stored
  at a path. Second call with the stored key yields the **same** ServiceID
  (fake server asserts the key blob is sent back).
- **Red:** `tor_add_onion_new_persists_key`; `tor_add_onion_reuse_key_same_id`.
- **Green:** helper `add_onion_persistent(datadir_file, virt, target)`.
- **Refactor:** key file mode 0600 on Unix.
- **Verify:** `cargo test -p rbitcoin-node tor_add_onion_`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 3 — `--tor-control` / cookie flags

- **Contract:** `--tor-control HOST:PORT` (omit ADDR → `127.0.0.1:9051`).
  `--tor-control-cookie PATH`. `--tor-control-password` or conf
  `tor_control_password=` (prefer cookie). If `--tor-control` is set and
  AUTH fails at start → **loud error** (do not silently skip HS). If the
  flag is unset, do not connect.
- **Red:** `tor_control_cli_defaults`; `tor_control_auth_fail_is_start_error`.
- **Green:** `apply_kv`; kebab CLI; `run_p2p` calls AUTH when configured.
- **Refactor:** none.
- **Verify:** `cargo test -p rbitcoin-node tor_control_cli`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 4 — Electrum HS when Electrum listen is on

- **Contract:** `--tor-control` + `--electrum-listen` ⇒ `ADD_ONION` virtual
  port = Electrum bind port (or 50001) targeting `127.0.0.1:<bound>`.
  INFO log contains `….onion`. Without `--electrum-listen`, no Electrum HS.
- **Red:** `electrum_hidden_service_add_onion_when_listening` — fake control
  + fake Electrum TCP; assert ADD_ONION Port line. Node config unit if a
  full `run_p2p` is too heavy: inject the helper.
- **Green:** `start_electrum_if_ready` in
  [`run.rs`](../../crates/rbitcoin-node/src/run.rs) after bind, call helper.
- **Refactor:** share with 05/07 via the same function signature.
- **Verify:** `cargo test -p rbitcoin-node electrum_hidden_`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 5 — `server.features.hosts`

- **Contract:** when an Electrum onion is configured, `server.features.hosts`
  is `{ "<id>.onion": { "tcp_port": N } }` not `{}`. No `ssl_port`. Without
  onion, stays `{}`.
- **Red:** `cargo test -p rbitcoin-electrum features_hosts_onion_tcp` —
  `ElectrumConfig` field for onion host+port; `server.features` JSON.
- **Green:** [`server.rs`](../../crates/rbitcoin-electrum/src/server.rs)
  `server.features` arm (today `"hosts": {}`). Pass onion from node into
  `ElectrumConfig`.
- **Refactor:** do not advertise P2P onion here (07).
- **Verify:** `cargo test -p rbitcoin-electrum features_hosts_`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 6 — OPERATOR

- **Contract:** how to point at system tor control/cookie; Electrum onion
  log line; RPC not on onion. COMPAT Electrum TLS row stays “external” /
  plain TCP (no Q-63 close).
- **Red:** none.
- **Green:** OPERATOR; optional one-line COMPAT hosts note if features
  change is user-visible.
- **Verify:** grep `--tor-control`.
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

## Test budget

Fake control server units. Electrum features unit. Avoid a full mainnet-shaped
`run_p2p` unless a slim regtest node test already exists and is cheap.

## Risks / follow-ups

Tor cookie path differs by distro. Do not glob the filesystem in production
beyond the configured path + a documented default. 05/07 must not copy-paste
AUTH.
