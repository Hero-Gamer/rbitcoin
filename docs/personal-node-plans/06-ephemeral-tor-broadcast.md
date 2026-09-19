# 06 — Ephemeral Tor broadcast (not Dandelion++)

## Goal

A locally submitted transaction (`sendrawtransaction`, Electrum
`transaction.broadcast`, Esplora `POST /tx` / package) is **not** INV’d on
the long-lived outbound set. After mempool **accept**, the node opens a
**new SOCKS circuit** (fresh isolation credentials from [00](./00-socks-proxy.md)),
BIP324-handshakes one or two peers, sends `tx`, disconnects.

This is **not** Dandelion++ (no stem graph, no fluff delay, no per-tx relay
state machine).

## Constraints

- Depends on **00** (SOCKS + `dial_isolated`). **02** optional (prefer an
  onion peer from AddrMan when `--onion` / `--proxy` is set).
- Standing outbound sessions must not announce that tx (INV / `tx` relay).
- `--blocks-only` stays: no relay of *others’* txs. This path is **local
  origin only**.
- Failure: log; keep the tx in mempool; **do not** fall back to standing-peer
  INV (that defeats the contract). Confirmation can still happen via blocks.
- Cap 1–2 one-shot peers.
- Prefer sending `tx` after handshake rather than INV/getdata if the peer
  will accept unsolicited `tx` (same as today’s submit announce shape,
  isolated). If the standing path only INVs, match that on the one-shot
  session only.

## Out of scope

Dandelion++. Changing libre admission. Broadcasting other peers’ txs.

## Steps

### Step 1 — Local-origin mark at accept

- **Contract:** txs accepted via RPC / Electrum / Esplora / `submitpackage`
  are tagged local-origin. P2P inbound `tx` is not. The tag is process-RAM
  (mempool entry / a side set), not a new store schema.
- **Red:** `cargo test -p rbitcoin-mempool local_origin_rpc_not_p2p` or hub
  test: RPC accept sets the flag; wire accept does not.
- **Green:** flag on the mempool entry or a `HashSet<Txid>` on the hub
  drained by the broadcast task.
- **Refactor:** one mark at the shared `accept` call sites (RPC, Electrum,
  Esplora already share hub accept).
- **Verify:** `cargo test -p rbitcoin-mempool local_origin_` ;
  `cargo test -p rbitcoin-rpc` sendraw if that is where the flag is set
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 2 — Standing peers skip local-origin INV

- **Contract:** with ephemeral broadcast enabled (on when `--proxy` or
  `--onion` is set, or an explicit `--broadcast-isolated` default-on with
  proxy), tip-follow does not INV/announce local-origin txs to the standing
  set. Other mempool txs still relay as today (unless `--blocks-only`).
- **Red:** `cargo test -p rbitcoin-net local_origin_not_inv_on_standing_peer`
  — two live sessions: submit locally; standing peer records **no** INV/tx
  for that txid; a P2P-originated tx still INVs (unless blocksonly).
- **Green:** announce filter in tx relay
  ([`tx_relay.rs`](../../crates/rbitcoin-net/src/tx_relay.rs) / peer out
  queue).
- **Refactor:** do not special-case Electrum vs RPC.
- **Verify:** `cargo test -p rbitcoin-net local_origin_not_inv`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 3 — One-shot isolated dial sends `tx`

- **Contract:** after accept of a local-origin tx, `dial_isolated` to 1–2
  targets, handshake, send the tx, disconnect. Fake SOCKS + fake v2 peer
  records exactly one `tx` (or INV+`tx`) and then EOF. Fresh SOCKS
  credentials ≠ the standing peer’s credentials (fake proxy records
  usernames).
- **Red:** `ephemeral_broadcast_one_shot_tx_and_new_socks_creds`.
- **Green:** task on the hub/node after accept. Target pick: onion from
  AddrMan if any; else IPv4/IPv6 via SOCKS. Require proxy; if no proxy,
  **do not enable** this path (standing INV remains — pin that in a unit:
  no proxy ⇒ today’s announce).
- **Refactor:** reuse 00 Dialer; no second SOCKS implementation.
- **Verify:** `cargo test -p rbitcoin-net ephemeral_broadcast_`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 4 — One-shot failure does not fall back to standing INV

- **Contract:** if isolated dial/handshake/send fails, log at WARN; tx stays
  in mempool; standing peers still do not INV it.
- **Red:** `ephemeral_broadcast_fail_does_not_inv_standing`.
- **Green:** error path; no “retry via relay”.
- **Refactor:** none.
- **Verify:** `cargo test -p rbitcoin-net ephemeral_broadcast_fail`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 5 — Electrum / Esplora / package hit the same path

- **Contract:** Electrum `transaction.broadcast` and Esplora `POST /tx` /
  `POST /txs/package` set local-origin (step 1) so they cannot bypass.
  Cross-surface test may extend
  [`cross_surface.rs`](../../crates/rbitcoin-test/tests/cross_surface.rs)
  only if cheap; otherwise unit the accept flag at each surface.
- **Red:** `electrum_broadcast_is_local_origin`; `esplora_post_tx_is_local_origin`.
- **Green:** if step 1 already covers shared accept, this step is Verify
  only — do not add a twin path.
- **Refactor:** delete any submit-specific announce in Electrum.
- **Verify:** `cargo test -p rbitcoin-electrum` / `rbitcoin-esplora` filters
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 6 — OPERATOR

- **Contract:** document that `--proxy` implies isolated local broadcast;
  not Dandelion++; failures stay in mempool without P2P INV.
- **Red:** none.
- **Green:** OPERATOR.
- **Verify:** grep.
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

## Test budget

Fake SOCKS + fake v2 peer. Do not require live Tor. Prefer net units over a
new multi-minute scenario.

## Risks / follow-ups

If `--blocks-only` + no proxy, local submit already admits without P2P
relay (`node_run_p2p_short`). Isolated broadcast **needs** proxy to hide
origin; without proxy, standing INV is honest (home IP). Dandelion++ stays
a follow-up, not a rename of this PR.
