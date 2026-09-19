# 05 — Electrum and Esplora onion (and I2P if cheap)

## Goal

Wallets reach **this node’s** Electrum and Esplora without a forwarded
clearnet port. Electrum HS is [03](./03-tor-control-hs.md). This PR adds
Esplora `ADD_ONION` (REST + `/ws` on the same TCP listener), surfaces both
onion URLs on `getnetworkinfo` (or logs + existing network RPC — **no** new
wallet RPC), and optionally I2P destinations if [04](./04-i2p.md) STREAM
FORWARD is already there.

SOCKS does **not** create these services. Clients use SOCKS to *reach*
onions. Operator fallback (document only): `torrc` `HiddenServicePort`.

## Constraints

- Depends on **03** (control helper + Electrum HS). **04** if I2P wallet
  forward is in this PR.
- Esplora is [`--esplora-listen`](../../crates/rbitcoin-esplora/src/server.rs)
  plain HTTP/WS. Onion is TCP to that loopback. No TLS/WSS in-process.
- Persist `{datadir}/onion/esplora.priv` separately from Electrum.
- RPC unix socket / `--rpc-listen` stays off onion.
- Default: when `--tor-control` and `--esplora-listen` are both set, create
  the Esplora HS (same “on if both configured” rule as Electrum in 03).
  `--esplora-onion=0` disables.

## Out of scope

In-binary TLS (rejected). SH allowlist. `--personal`. P2P `--listen-onion` (07).

## Steps

### Step 1 — Esplora `ADD_ONION`

- **Contract:** `--tor-control` + `--esplora-listen` ⇒ `ADD_ONION` virtual
  HTTP port → `127.0.0.1:<esplora>`. Persist esplora key. INFO log
  `http://….onion:<port>` (and that `/ws` is the same port). `--esplora-onion=0`
  skips.
- **Red:** `esplora_hidden_service_add_onion` — fake control records Port=
  targeting the Esplora bind.
- **Green:** `start_esplora_if_ready` in
  [`run.rs`](../../crates/rbitcoin-node/src/run.rs) after bind; reuse 03
  helper.
- **Refactor:** do not clone AUTH; one `OnionServices` struct holding
  electrum/esplora/p2p ids.
- **Verify:** `cargo test -p rbitcoin-node esplora_hidden_`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 2 — `getnetworkinfo` (or equivalent) lists wallet onions

- **Contract:** when HS exist, JSON-RPC `getnetworkinfo` includes them
  without a new RPC family. Prefer `localaddresses` rows with the onion
  hostname and port, and/or a small extra object `onions: { electrum, esplora }`
  **only if** `localaddresses` cannot represent non-IP. Do not add
  `getwalletconnect`.
- **Red:** `cargo test -p rbitcoin-rpc getnetworkinfo_includes_electrum_onion`
  — hub/config inject onion strings.
- **Green:** [`methods`](../../crates/rbitcoin-rpc) `getnetworkinfo`; node
  fills from HS helper.
- **Refactor:** Electrum `features.hosts` already from 03; keep one source
  of truth for the hostname.
- **Verify:** `cargo test -p rbitcoin-rpc getnetworkinfo_`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 3 — I2P wallet forward (only if 04 helper exists)

- **Contract:** If 04 exported STREAM FORWARD to an arbitrary loopback
  port, add Electrum and Esplora I2P destinations (persist under
  `{datadir}/i2p/electrum.priv` etc.) and log the destination. If 04 did
  not, **skip this step** (do not invent a second SAM client).
- **Red:** `electrum_i2p_forward_when_sam_incoming` (skip/omit if no helper).
- **Green:** same FORWARD as P2P, different virt port.
- **Refactor:** n/a if skipped.
- **Verify:** `cargo test -p rbitcoin-node i2p_wallet_` or omit.
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed, or skipped in the PR body because 04 has no STREAM FORWARD helper

### Step 4 — OPERATOR / COMPAT + NixOS module

- **Contract:** OPERATOR: Electrum + Esplora onion URLs, Sparrow
  `tcp://….onion:port`, no nginx required for onion. COMPAT Electrum TLS
  row remains reverse-proxy for **clearnet**; onion is plain TCP. Do not
  close Q-63 (TLS still parked). Module: `esplora.hiddenService` reusing 03
  tor control. Eval asserts the Esplora ADD_ONION-related argv. Runtime
  label only if new systemd deps land here (prefer 03).
- **Red:** eval assert for esplora hidden-service flag.
- **Green:** module + eval + OPERATOR + COMPAT one-liner if hosts/onions are
  user-visible.
- **Verify:** `nix build .#checks.x86_64-linux.nixos-module-eval --no-link`;
  grep onion.
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

## Test budget

Fake control + RPC unit. No live Tor. No Sparrow in CI.

## Risks / follow-ups

Esplora clients that require HTTPS will not work on onion without TLS
(rejected). Document tcp/http onion. P2P onion is 07, separate keys.
