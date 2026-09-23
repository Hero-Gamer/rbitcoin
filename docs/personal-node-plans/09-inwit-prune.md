# 09 — Prune inputs and witnesses (`seqsigwit`); advertise `NODE_NETWORK_LIMITED`

## Goal

A home node can stop serving **Class A seqsigwit** (scriptSig + witness + input
prevout encoding) below a 288-height watermark. Unpruned nodes keep
`seqsigwit.body`. Pruned nodes serve only the kept window, while still:

1. Confirming new blocks (each connected height is recorded in the window).
2. Reorging within the kept window.
3. Serving the last **288 heights** on P2P (BIP159 **`NODE_NETWORK_LIMITED`**).
4. Serving wallet **scripthash / UTXO / status / outspend / vout** paths that
   only need `txout` + `spent` + `txid.body` + headers.

This is **not** Core `-prune` of entire `blk*.dat` files. We keep headers,
`txout`, `spent`, `txid.body`, SH, tweaks. We drop old **inputs and
witnesses**.

**Layout:** two modes, not a rolling `seqsigwit.body`.

- **Unpruned** (default): Class A `seqsigwit.body` is the witness archive.
  Reconstruct, getdata, and wire RPC read it.
- **`--prune-seqsigwit`:** witness below the watermark is not served. The last
  **288 heights** are one file each, `store/seqsigwit.window/{height}.bin`, plus
  a RAM cache of those heights capped by
  `--prune-seqsigwit-ram-threshold-bytes` (default 256 MiB). **`0` keeps nothing
  in RAM** — every height, including tiny IBD blocks, is read back from its
  file. `pruneheight = tip - 288` once `tip > 288`. Kept heights are
  `h > pruneheight`. A reorg at a kept height replaces that height's file.
  Disconnect at or below `pruneheight` fails closed. No `SCHEMA_VERSION`
  bump: the mode is the `{store}/seqsigwit.prune` sidecar.

**JSON vs wire:** serve **honest partial objects** (vout/status/txids we still
have). Refuse **wire** (hex/raw/P2P block/tx) rather than invent vin, fee,
size, weight, or a BIP144 body.

## `NODE_NETWORK_LIMITED` (you are not wrong)

BIP159 / Core:

| Flag | Bit | Meaning |
|------|-----|---------|
| `NODE_NETWORK` | 1 | I can serve **historical** blocks (full archive). |
| `NODE_NETWORK_LIMITED` | 1024 (bit 10) | I can serve at least the **last 288 blocks** (~2 days at 10 m). |

They are alternatives: a pruned Core node advertises `NETWORK_LIMITED|WITNESS`,
**not** `NETWORK`. 288 is `MIN_BLOCKS_TO_KEEP` on the **active chain**. Our
store window is the same 288 **heights** behind tip; orphan / stale blocks
at those heights are extra seqsigwit, not a reason to drop below 288 heights.
A different constant,
[`NODE_NETWORK_LIMITED_ALLOW_CONN_BLOCKS` = 144](../../crates/rbitcoin-net/src/peer.rs),
is only for **outbound selection** (do not IBD-deep from a limited peer). Do
**not** use 144 as our keep window.

Today we advertise `NETWORK|WITNESS|P2P_V2` and tests pin **no**
`NETWORK_LIMITED` ([`methods_tests.rs`](../../crates/rbitcoin-rpc/src/methods_tests.rs)
`we do not advertise NETWORK_LIMITED`). After this plan, a pruned node
advertises `NETWORK_LIMITED|WITNESS|P2P_V2`. DNS/seed **desire** for IBD still
asks `NETWORK` (full) peers; VERSION bits we *offer* are limited.

## Kept window (288 height files + RAM)

Unpruned nodes read `seqsigwit.body`. Pruned nodes serve witness only from the
window:

- Confirm writes `store/seqsigwit.window/{height}.bin` for the connected height.
- RAM holds those heights while the encoded input bytes stay under
  `--prune-seqsigwit-ram-threshold-bytes`. **`0` skips RAM** and every lookup
  reads the height file.
- When the tip moves, files with `height <= pruneheight` are removed.
  `pruneheight` is `tip - 288` (a height, not a count of files).
- Enabling prune on an archive that already has `seqsigwit.body` copies the kept
  heights into the window once. After that, serving does not depend on a
  rolling stem. A datadir that already has `seqsigwit.prune` and is opened
  without `--prune-seqsigwit` refuses to start.
- `--datadir-cold` still places `seqsigwit.body` on the cold path. The window
  stays under the hot store next to `seqsigwit.prune`.

Do not punch `seqsigwit.body`, and do not replace this window with a rolling
segment log. The operator choice is the full stem, or the 288-height window.

## Partial JSON vs 404 / `pruned`

We still have, for every confirmed create: `txid.body`, LAYOUT17 **txout**
(version, locktime, `input_count`, every output value/script), `spent`,
headers. We **do not** have prevout edges, scriptSig, witness — so we cannot
compute fee, size, weight, wtxid, or a valid tx.

**Never invent** `vin: []`, zero fee, or a truncated hex. Empty vin looks like
a coinbase; fake hex fails wallet verify.

| Kind | Serve? | Why |
|------|--------|-----|
| **Wire** (Electrum hex, Esplora `/hex` `/raw`, RPC `getrawtransaction` / `getblock` v0/v2, P2P `getdata` block/tx, BIP37 merkleblock) | **Refuse** | Not a Bitcoin tx without seqsigwit. |
| **Object fields we still own** (status, vout from txout, txids, outspend from `spent`, header JSON, SH history/utxo) | **Serve** | Independently true; wallets use these more than hex. |
| **vin / fee / size / weight / sigops / hex** | **Omit** (JSON) or refuse (wire) | Need seqsigwit. Omit the keys; do not null them into `[]` / `0`. |

Esplora JSON may add `"pruned": true` (extra key, same idea as Electrum
`chain_tip`). Electrum `transaction.get` has no honest partial: both modes
are a BIP144 hex or verbose-**plus-hex**. Return error **`pruned`**, not a
vout-only object that still claims to be `transaction.get`.

## Constraints

- **No silent wipe.** Durable `pruneheight` in store meta / sidecar. Same
  commit as the format code ([`SCHEMA.md`](../../SCHEMA.md) bump **or** a
  named sidecar with refuse-on-mismatch).
- Keep **288 heights** of witness (`pruneheight = tip - 288` once the tip is
  above that). The window is those height files and the RAM cache, not a
  count of orphan blocks.
- Reorg that would disconnect **at or below** `pruneheight` → fail closed
  (Core: cannot reorg pruned). Do not invent undo from `spent` alone.
- Confirm still records the height file (and RAM, unless the cap is 0) as
  the block connects. Dropping a height is deleting `{height}.bin` once it
  falls out of the window.
- COMPAT “Pruning / GUI | Not supported” becomes “seqsigwit prune / NETWORK_LIMITED;
  not Core `-prune` of headers/txout”.

## Out of scope

Dropping `txout` / `spent` / headers / SH (that would break wallets).
AssumeUTXO. BIP157. Core GUI. Pruning during IBD before the block is
connected. Arti/Tor (00–08). Thin-seqsigwit (keep prevout refs, drop
scriptSig/witness) — that would unlock fee/vin-without-witness; not this
plan.

## API contract (keep vs fail)

Heights **above** the watermark behave as today. Below: table.

### P2P

| Path | Pruned height | Kept window (last ≥288 **heights**) |
|------|---------------|-------------------------|
| `getdata` `MSG_WITNESS_BLOCK` / `MSG_BLOCK` / `MSG_CMPCT_BLOCK` | `notfound` (same as missing) | Reconstruct and serve as today ([`encode_served_witness_block`](../../crates/rbitcoin-net/src/peer.rs)) |
| `getblocktxn` | n/a (tip compact only) | Serve if the block is still in the window / cache |
| Headers / `getheaders` | Unchanged | Unchanged |
| VERSION `services` | — | Advertise `NETWORK_LIMITED\|WITNESS\|P2P_V2`, **not** `NETWORK` |
| Compact **receive** / mempool | Unchanged (wire + mempool, not historical seqsigwit) | Unchanged |

### JSON-RPC ([`docs/rpc.md`](../rpc.md))

| Method | Pruned (seqsigwit gone) | Notes |
|--------|---------------------|--------|
| `getblockchaininfo` | `pruned: true`, `pruneheight: N` | Today hardcoded `pruned: false` |
| `getblock` verbosity **0** (raw) | `-8` `Block not available (pruned data)` | Core needle |
| `getblock` verbosity **1** | **Keep** (`block_txids` / `txid.body`) | txid list, no vin |
| `getblock` verbosity **2** | `-8` pruned | Full txs need seqsigwit |
| `getblockheader` / `getblockhash` / `getblockcount` | Keep | Headers only |
| `getblockstats` | serve from `txstat` when stamped, else `-8` | Reconstruct + lazy-stamp if unstamped and seqsigwit remains |
| `getrawtransaction` | **`-8` `Transaction not available (pruned data)`** | Not `-5` unknown txid |
| `decoderawtransaction` | n/a (client hex) | |
| `gettxout` | **Keep** | Class A outs + spent |
| `scantxoutset` | **Keep** | txout + spent |
| `getindexinfo` / `getchaintips` | Keep | |
| `getblocktemplate` / mempool RPCs | Keep | Tip + mempool |
| `getnetworkinfo.localservicesnames` | Includes `NETWORK_LIMITED`, omits `NETWORK` | Flip today’s pin |

### Electrum

| Method | Pruned | Notes |
|--------|--------|-------|
| `transaction.get` (hex or verbose) | JSON-RPC error **`pruned`** (not `tx not found`) | Both shapes need a wire body. Do not return vin-less verbose. |
| `transaction.get_merkle` / `id_from_pos` | **Keep** | Merkle from txids + headers |
| `scripthash.get_history` / `get_balance` / `listunspent` / subscribe | **Keep** | SH + txout + spent; history rows have no witness |
| `blockchain.block.header(s)` | Keep | |
| `tweaks.subscribe` / `silentpayments.subscribe` | **Keep** if serve is txout-only (indexed path). Naive parent-seqsigwit peek below watermark → skip/omit that height’s spend-side fields, do not Corrupt | Pin in tests |
| `transaction.broadcast` | Keep | Mempool |

### Esplora REST (shipped + [#632](https://github.com/reardencode/rbitcoin/pull/632) 0.8 drop-in)

[#632](https://github.com/reardencode/rbitcoin/pull/632) adds electrs HTTP
(`/internal/*`, `?after_txid=`, `POST /addresses|scripthashes/txs`,
`GET /broadcast`, `POST /txs/test`, tx JSON `sigops`, unix listen).

**Partial tx object** (when seqsigwit is gone): `txid`, `version`, `locktime`,
`vout[]` from txout, `status`, `"pruned": true`. **Omit** `vin`. Fill `fee` /
`size` / `weight` from stamped `txstat` when those rows exist;
unstamped pruned omits those keys too. Same shape on list rows so `/txs`
paging matches `/txs/summary` (do not drop the txid from the page).

| Route | Pruned confirmed tx/block | Why |
|-------|---------------------------|-----|
| `GET /tx/:txid` JSON | **200** partial object; `fee`/`size`/`weight` from `txstat` when stamped | vout+status still true; still omit `vin` |
| `GET /tx/:txid/hex`, `/raw` | **404** body `pruned` | Wire |
| `GET /tx/:txid/status` | **Keep** (200) | Header + fk, no seqsigwit |
| `GET /tx/:txid/merkle-proof` | **Keep** | txids |
| `GET /tx/:txid/merkleblock-proof` | **404** | BIP37 needs full txs |
| `GET /tx/:txid/outspend(s)` | **Keep** | `spent` slot `vin` index, not seqsigwit of the spent tx |
| `GET /block/:hash` JSON | **200**; omit `size`/`weight` (or only if we cannot compute them); `"pruned": true` | Header + txids; not a witness size |
| `GET /block/:hash/raw` | **404** | Full witness block |
| `GET /block/:hash/header` `/status` `/txids` `/txid/:i` | **Keep** | header + `txid.body` |
| `GET /block/:hash/txs` (public 25/page and **632** `GET /internal/block/:hash/txs`) | **200** pages of **partial** tx objects | Same omit-vin rule; stamped `txstat` fills fee/size/weight |
| Address `/` stats, `/utxo`, `/txs/summary` | **Keep** | SH + values from txout |
| Address `/txs`, `/txs/chain`, **632** `POST /addresses/txs` | **200** with partial rows for pruned txs | Keep paging aligned with summary |
| **632** `?after_txid=` | Keep (txid cursor) | |
| **632** `POST /internal/txs` | Full JSON when in window; **partial** (not omitted) when pruned; unknown still omitted | Distinct from missing |
| **632** `POST /internal/txs/outspends/*` | **Keep** | spent table |
| **632** `GET /broadcast`, `POST /txs/test`, mempool `/internal/mempool/*` | Keep | No archive seqsigwit |
| WS `address-transactions` / `block-transactions` | Partial object if reconstruct fails; do not send `vin: []` | Mempool path unchanged |

## Steps

### Step 1 — `Pruned` vs `Corrupt` on reconstruct

- **Contract:** `get_tx_full` / `tx_wire_bytes` / `reconstruct_block_*` /
  `witness_block_bytes_by_hash` return a **named** `QueryError::Pruned {
  height }` when `height <= pruneheight`. Hole above watermark is still
  `Corrupt("invariant: …")`. `block_txids` still works. `txout` get still
  works below watermark.
- **Red:** `cargo test -p rbitcoin-query reconstruct_pruned_returns_pruned_not_corrupt`
  — tiny `/tmp` chain, set watermark, drop or stub seqsigwit; reconstruct below
  fails Pruned; above succeeds; `block_txids` both sides; outs still readable.
- **Green:** watermark on Query/store (test stub first, durable in step 2).
- **Refactor:** one helper `seqsigwit_available(fk) -> Result<bool>`.
- **Verify:** `cargo test -p rbitcoin-query reconstruct_pruned_`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 2 — Durable pruneheight + 288 height files

- **Contract:** `--prune-seqsigwit` / conf (default **off**). `{store}/seqsigwit.prune`
  is a 4-byte LE `pruneheight` (`u32::MAX` = on, nothing dropped yet; missing
  file = off). After the tip passes 288 heights, `pruneheight = tip - 288`.
  Kept witness is `store/seqsigwit.window/{height}.bin` plus the RAM cache.
  `--prune-seqsigwit-ram-threshold-bytes 0` writes every height and retains none
  in RAM. No schema bump. Reopen restores the sidecar. A pruned datadir
  opened without the flag refuses to start. Enabling prune on an existing
  archive seeds the window from `seqsigwit.body` for the kept heights.
- **Red:** `prune_watermark_survives_reopen`;
  `prune_ram_window_drops_fks_on_disconnect_and_replace`;
  `prune_ram_threshold_zero_spills_tiny_blocks`;
  spill symlink outside the window is `Corrupt`.
- **Green:** sidecar + height files + RAM cache.
- **Refactor:** no rolling `seqsigwit.body`, no punch path on this table.
- **Verify:** `cargo test -p rbitcoin-query prune_`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 3 — Reorg below pruneheight fail-closed

- **Contract:** disconnect/reorg whose rewind height `<= pruneheight` is
  `StoreError` / hub reject, not a silent spent undo. Tip-follow does not
  apply that branch.
- **Red:** `cargo test -p rbitcoin-query reorg_through_pruneheight_refuses`.
- **Green:** `connect` / rewind checks watermark.
- **Verify:** that test + existing reorg tests still pass above watermark.
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 4 — Advertise `NETWORK_LIMITED`, serve/notfound

- **Contract:** pruned process: `local_service_flags()` =
  `NETWORK_LIMITED|WITNESS|P2P_V2`. Unpruned unchanged (`NETWORK|…`).
  `getdata` for a block with height `<= pruneheight` → `notfound`. Last 288
  **heights** still reconstruct-serve (including orphan hashes still in that
  height window). RPC `getnetworkinfo.localservicesnames` matches.
  DNS **seed query** still uses `NETWORK|WITNESS|P2P_V2` (find archive
  peers for IBD).
- **Red:** flip
  `we do not advertise NETWORK_LIMITED` when prune on; add
  `pruned_getdata_old_block_notfound` (seeder pruned, peer asks height 1
  after 300-pad — use tiny heights: keep=2 in tests).
- **Green:** `local_service_flags` takes prune mode;
  [`encode_served_witness_block`](../../crates/rbitcoin-net/src/peer.rs)
  maps `Pruned` → `None` → existing notfound.
- **Refactor:** do not conflate seed-required flags with advertised flags.
- **Verify:** `cargo test -p rbitcoin-rpc localservices` ;
  `cargo test -p rbitcoin-net pruned_getdata_`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 5 — RPC / Electrum / Esplora needles

- **Contract:** tables above. Wire surfaces refuse (RPC `-8` Core text,
  Electrum `pruned`, Esplora `/hex` `/raw` 404). Esplora **JSON** is 200
  partial (`pruned: true`, vout+status, **no** `vin` key). Address `/txs`
  keeps the row. `POST /internal/txs` returns partial, not omit, for pruned
  ids.
- **Red:** `getblock_pruned_minus8`; `getrawtransaction_pruned`;
  `electrum_transaction_get_pruned`; `esplora_tx_json_partial_omits_vin`;
  `esplora_tx_raw_404_status_200`; `esplora_outspend_still_200`;
  `esplora_address_txs_includes_pruned_row` (632-compatible).
- **Green:** map `QueryError::Pruned` at each surface; one `tx_json` builder
  branch that fills vout from `txout` without reconstruct.
- **Refactor:** one `is_pruned` at Query, not ad-hoc `contains("not found")`.
- **Verify:** `cargo test -p rbitcoin-rpc pruned_` ;
  `cargo test -p rbitcoin-electrum pruned_` ;
  `cargo test -p rbitcoin-esplora pruned_`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 6 — OPERATOR / COMPAT / SCHEMA / rpc.md + NixOS module

- **Contract:** OPERATOR `--prune-seqsigwit`, 288-height files under
  `seqsigwit.window/`, RAM cap (`0` = files only), `NETWORK_LIMITED`, cannot
  reorg through pruneheight. COMPAT prune row.
  `getblockchaininfo` fields. SCHEMA/sidecar bytes. Partial Esplora JSON.
  Do not copy this file into quality.md until scheduled. Module:
  `pruneSeqSigWit`; `coldDataDir` already exists. Eval asserts `--prune-seqsigwit`.
  Runtime label only if tmpfiles / `ReadWritePaths` change.
- **Red:** eval assert for `--prune-seqsigwit`.
- **Green:** module + eval + those doc owners.
- **Verify:** `nix build .#checks.x86_64-linux.nixos-module-eval --no-link`;
  grep `NETWORK_LIMITED`, `prune-seqsigwit`, `seqsigwit.window`.
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

## Test budget

Tiny `/tmp` chains (production keep is 288 heights). No mainnet open. One
P2P notfound + reconstruct-serve of a kept tip. Height-file removal runs on
every OS.

## Risks / follow-ups

- Tweaks naive path parent seqsigwit: must not Corrupt on pruned parents.
- mempool.space **frontend** may assume `vin` always present on `GET /tx`;
  personal-node wallets (Electrum history/utxo) do not. Partial JSON is for
  honest objects, not a claim that we are a full electrs archive.
- Core functional prune scripts stay skip until we claim Core `-prune`
  (we do not).
- Thin-seqsigwit (keep prevout fk+vout, drop scripts) would allow fee + vin
  without witness; separate plan if a wallet needs that.
