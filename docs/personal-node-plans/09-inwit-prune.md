# 09 — Prune inputs and witnesses (`inwit`); advertise `NODE_NETWORK_LIMITED`

## Goal

A home node can drop **Class A `inwit` (scriptSig + witness + input prevout
encoding)** for creates below a prune watermark, reclaiming the ~486 GiB cold
stem ([`SCHEMA.md`](../../SCHEMA.md)), while still:

1. Confirming new blocks (IBD and tip write `inwit` first, drop later).
2. Reorging within the kept window.
3. Serving the last **288 heights** on P2P (BIP159 **`NODE_NETWORK_LIMITED`**).
4. Serving wallet **scripthash / UTXO / status / outspend / vout** paths that
   only need `txout` + `spent` + `txid.body` + headers.

This is **not** Core `-prune` of entire `blk*.dat` files. We keep headers,
`txout`, `spent`, `txid.body`, SH, tweaks. We drop old **inputs and
witnesses**.

**Layout:** a **rolling 288-height inwit stem** (append a segment, unlink the
oldest). Not `FALLOC_FL_PUNCH_HOLE` on the genesis-length `inwit.body`.
Orphan / stale / side-branch blocks at kept heights mean **more than 288
inwit blocks** on disk — the pin is 288 **heights** behind tip, not 288
records.

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
at those heights are extra inwit, not a reason to drop below 288 heights.
A different constant,
[`NODE_NETWORK_LIMITED_ALLOW_CONN_BLOCKS` = 144](../../crates/rbitcoin-net/src/peer.rs),
is only for **outbound selection** (do not IBD-deep from a limited peer). Do
**not** use 144 as our keep window.

Today we advertise `NETWORK|WITNESS|P2P_V2` and tests pin **no**
`NETWORK_LIMITED` ([`methods_tests.rs`](../../crates/rbitcoin-rpc/src/methods_tests.rs)
`we do not advertise NETWORK_LIMITED`). After this plan, a pruned node
advertises `NETWORK_LIMITED|WITNESS|P2P_V2`. DNS/seed **desire** for IBD still
asks `NETWORK` (full) peers; VERSION bits we *offer* are limited.

## Rolling inwit window (not punch)

### Why punch is the wrong tool here

`inwit.body` is one append-only Class A stem. Loc is **stride-8** and records
are packed (empty inwit is an **8-byte** zero pad;
[`SCHEMA.md`](../../SCHEMA.md)). `TableFile::zero_range` already has Linux
`FALLOC_FL_PUNCH_HOLE` (`KEEP_SIZE`).

That primitive is a poor reclaim path for this table:

| Fact | Consequence |
|------|-------------|
| Punch is **filesystem-block aligned** (4 KiB typical) | A few-hundred-byte inwit record shares a page with neighbors. Per-tx (or even per-small-block) punch **cannot** free one create without eating live bytes next to it, or else no-ops on a sub-block range. |
| One 486 GiB file, monotone `inwit.loc` abs | Per-record punch would split the extent tree into millions of holes. Journal + `fiemap` bloat; `stat`/`cp`/`tar` see a still-huge sparse file. |
| SSD cost | Punch is TRIM of those extents, not a rewrite of the payload, so NAND write amp is not “rewrite 486 GiB”. The churn is **FS metadata** (extent map, journal) and FTL mapping updates. Coarse one-shot prefix punch is tolerable; **per-tx punch is not**. |
| Logical size stays 486 GiB | Sparse `KEEP_SIZE` never shrinks `st_size`. Backups and NAS copies often expand the holes. |
| Home-node IBD with prune on | We must **never allocate** 486 GiB in the first place. Punching a file we should not have grown is the wrong shape. |

A single aligned prefix punch of `[0, first_kept_off)` **after** a full archive
would reclaim `st_blocks` in one syscall. That is still a 486 GiB sparse inode
forever, and it does not help prune-from-genesis. **Do not ship that as the
production layout.**

### Contract: append tail, unlink head

Keep ≥ **288 connected heights** of inwit as a **small rolling log**
(watermark is height, not a count of inwit blocks):

- Confirm still encodes full inwit on the Class A write thread and **appends**
  to the current segment (fallocate grow + pwrite + publish HWM — same
  `TableFile` discipline as today).
- When **every** height in a segment is `≤ pruneheight`, **unlink** the file
  (background; not the confirm hot path). The OS drops the inode and TRIMs
  the whole object. One create, one sequential write, one unlink per dropped
  segment. A stale sibling at a still-kept height keeps the file.
- Window size on disk: ~486 GiB / ~900k blocks × 288 ≈ **150–400 MiB** of
  inwit on a linear chain (witness era toward the high end). Orphans at kept
  heights add more (see below). A few files, not a sparse 486 GiB stem.

**288 heights ≠ 288 inwit blocks.** `pruneheight` is
`tip.saturating_sub(288 + buffer)` — a **height**. BIP159 / reorg need inwit
for every create whose **connected height** is `> pruneheight`. A reorg
leaves the disconnected (orphan / stale / side-branch) block’s inwit in
Class A until that *height* falls out of the window. Two blocks at the same
kept height are two inwit payloads. The rolling stem therefore often holds
**more than 288 blocks of inwit** in order to keep **288 heights** behind
tip. Do **not** unlink because “we already have 288 inwit blocks” while a
kept height still has a stale sibling. Do not size the window by counting
active-chain blocks only.

**Segment grain (pin in SCHEMA, not both):** size-capped sealed files
(**64 MiB** target, or the current height if a single block is larger), not
one inode per height (288 main-chain heights is fine for the kernel; fewer
files is less open/stat noise on reconstruct of a whole block). Unlink only
when **every** height in the file is `≤ pruneheight`, including orphan
heights in that file (may hold up to ~64 MiB extra).

**Loc:** SCHEMA bump. Global monotone abs into one `inwit.body` cannot
survive unlink of the prefix. Below watermark: loc **sentinel** (not a live
span) so `get_tx_full` is `QueryError::Pruned` rather than `Corrupt`. Kept
window: loc is **segment id + local off** (or height → segment map + per-file
loc). Unexpected hole **inside** the window → `Corrupt("invariant: …")`.

**Enable on an existing archive:** sequential **copy every inwit whose
height is `> pruneheight`** (active chain **and** orphans in that height
window) from the old `inwit.body` into the rolling stem, then **unlink** the
genesis-length file. Do not punch it. A linear-chain copy is ~200 MiB; extra
stale blocks in the window add more. Refuse leftover single-stem
`inwit.body` without the new sidecar/schema (same commit as the format
code).

**Prune-from-genesis / IBD:** write only the rolling stem; never grow a
historical body. Peak inwit bytes ≈ height window (plus orphan inwit at
those heights) + one in-flight segment.

**`--datadir-cold`:** rolling files live with today’s cold inwit, not the hot
txout/spent dir.

**Non-Linux:** unlink is portable. No punch fallback, no 1 MiB zero-fill of
old records.

## Partial JSON vs 404 / `pruned`

We still have, for every confirmed create: `txid.body`, LAYOUT17 **txout**
(version, locktime, `input_count`, every output value/script), `spent`,
headers. We **do not** have prevout edges, scriptSig, witness — so we cannot
compute fee, size, weight, wtxid, or a valid tx.

**Never invent** `vin: []`, zero fee, or a truncated hex. Empty vin looks like
a coinbase; fake hex fails wallet verify.

| Kind | Serve? | Why |
|------|--------|-----|
| **Wire** (Electrum hex, Esplora `/hex` `/raw`, RPC `getrawtransaction` / `getblock` v0/v2, P2P `getdata` block/tx, BIP37 merkleblock) | **Refuse** | Not a Bitcoin tx without inwit. |
| **Object fields we still own** (status, vout from txout, txids, outspend from `spent`, header JSON, SH history/utxo) | **Serve** | Independently true; wallets use these more than hex. |
| **vin / fee / size / weight / sigops / hex** | **Omit** (JSON) or refuse (wire) | Need inwit. Omit the keys; do not null them into `[]` / `0`. |

Esplora JSON may add `"pruned": true` (extra key, same idea as Electrum
`chain_tip`). Electrum `transaction.get` has no honest partial: both modes
are a BIP144 hex or verbose-**plus-hex**. Return error **`pruned`**, not a
vout-only object that still claims to be `transaction.get`.

## Constraints

- **No silent wipe.** Durable `pruneheight` in store meta / sidecar. Same
  commit as the format code ([`SCHEMA.md`](../../SCHEMA.md) bump **or** a
  named sidecar with refuse-on-mismatch).
- Keep **≥ 288 heights** of inwit (`tip - pruneheight >= 288`, plus a small
  reorg buffer — Core keeps extra; pin **288 min heights**, extra is operator
  `--prune-buffer` default 0 or 144). The **number of inwit blocks** in that
  window is **≥ 288** and **greater** when orphan / stale blocks share those
  heights. Watermark and unlink are by height, not by block count.
- Reorg that would disconnect **at or below** `pruneheight` → fail closed
  (Core: cannot reorg pruned). Do not invent undo from `spent` alone.
- Confirm/IBD still **writes** full inwit. Drop is a background unlink after
  tip. Named `ibd: perf` timer if the walker joins write — prefer a
  **non-write-thread** unlink so no timer; if it takes the Class A appender,
  add the timer in the same commit.
- COMPAT “Pruning / GUI | Not supported” becomes “inwit prune / NETWORK_LIMITED;
  not Core `-prune` of headers/txout”.

## Out of scope

Dropping `txout` / `spent` / headers / SH (that would break wallets).
AssumeUTXO. BIP157. Core GUI. Pruning during IBD before the block is
connected. Arti/Tor (00–08). Thin-inwit (keep prevout refs, drop
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
| Compact **receive** / mempool | Unchanged (wire + mempool, not historical inwit) | Unchanged |

### JSON-RPC ([`docs/rpc.md`](../rpc.md))

| Method | Pruned (inwit gone) | Notes |
|--------|---------------------|--------|
| `getblockchaininfo` | `pruned: true`, `pruneheight: N` | Today hardcoded `pruned: false` |
| `getblock` verbosity **0** (raw) | `-8` `Block not available (pruned data)` | Core needle |
| `getblock` verbosity **1** | **Keep** (`block_txids` / `txid.body`) | txid list, no vin |
| `getblock` verbosity **2** | `-8` pruned | Full txs need inwit |
| `getblockheader` / `getblockhash` / `getblockcount` | Keep | Headers only |
| `getblockstats` | `-8` / existing `block body not in store` | Reconstruct |
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
| `tweaks.subscribe` / `silentpayments.subscribe` | **Keep** if serve is txout-only (indexed path). Naive parent-inwit peek below watermark → skip/omit that height’s spend-side fields, do not Corrupt | Pin in tests |
| `transaction.broadcast` | Keep | Mempool |

### Esplora REST (shipped + [#632](https://github.com/reardencode/rbitcoin/pull/632) 0.8 drop-in)

[#632](https://github.com/reardencode/rbitcoin/pull/632) adds electrs HTTP
(`/internal/*`, `?after_txid=`, `POST /addresses|scripthashes/txs`,
`GET /broadcast`, `POST /txs/test`, tx JSON `sigops`, unix listen).

**Partial tx object** (when inwit is gone): `txid`, `version`, `locktime`,
`vout[]` from txout, `status`, `"pruned": true`. **Omit** `vin`, `fee`,
`size`, `weight`, `sigops`. Same shape on list rows so `/txs` paging matches
`/txs/summary` (do not drop the txid from the page).

| Route | Pruned confirmed tx/block | Why |
|-------|---------------------------|-----|
| `GET /tx/:txid` JSON | **200** partial object | vout+status still true; explorer UIs that require `vin` fail closed on missing key (better than a fake coinbase) |
| `GET /tx/:txid/hex`, `/raw` | **404** body `pruned` | Wire |
| `GET /tx/:txid/status` | **Keep** (200) | Header + fk, no inwit |
| `GET /tx/:txid/merkle-proof` | **Keep** | txids |
| `GET /tx/:txid/merkleblock-proof` | **404** | BIP37 needs full txs |
| `GET /tx/:txid/outspend(s)` | **Keep** | `spent` slot `vin` index, not inwit of the spent tx |
| `GET /block/:hash` JSON | **200**; omit `size`/`weight` (or only if we cannot compute them); `"pruned": true` | Header + txids; not a witness size |
| `GET /block/:hash/raw` | **404** | Full witness block |
| `GET /block/:hash/header` `/status` `/txids` `/txid/:i` | **Keep** | header + `txid.body` |
| `GET /block/:hash/txs` (public 25/page and **632** `GET /internal/block/:hash/txs`) | **200** pages of **partial** tx objects | Same omit-vin rule; do not 404 the whole block list |
| Address `/` stats, `/utxo`, `/txs/summary` | **Keep** | SH + values from txout |
| Address `/txs`, `/txs/chain`, **632** `POST /addresses/txs` | **200** with partial rows for pruned txs | Keep paging aligned with summary |
| **632** `?after_txid=` | Keep (txid cursor) | |
| **632** `POST /internal/txs` | Full JSON when in window; **partial** (not omitted) when pruned; unknown still omitted | Distinct from missing |
| **632** `POST /internal/txs/outspends/*` | **Keep** | spent table |
| **632** `GET /broadcast`, `POST /txs/test`, mempool `/internal/mempool/*` | Keep | No archive inwit |
| WS `address-transactions` / `block-transactions` | Partial object if reconstruct fails; do not send `vin: []` | Mempool path unchanged |

## Steps

### Step 1 — `Pruned` vs `Corrupt` on reconstruct

- **Contract:** `get_tx_full` / `tx_wire_bytes` / `reconstruct_block_*` /
  `witness_block_bytes_by_hash` return a **named** `QueryError::Pruned {
  height }` when `height <= pruneheight`. Hole above watermark is still
  `Corrupt("invariant: …")`. `block_txids` still works. `txout` get still
  works below watermark.
- **Red:** `cargo test -p rbitcoin-query reconstruct_pruned_returns_pruned_not_corrupt`
  — tiny `/tmp` chain, set watermark, drop or stub inwit; reconstruct below
  fails Pruned; above succeeds; `block_txids` both sides; outs still readable.
- **Green:** watermark on Query/store (test stub first, durable in step 2).
- **Refactor:** one helper `inwit_available(fk) -> Result<bool>`.
- **Verify:** `cargo test -p rbitcoin-query reconstruct_pruned_`
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

### Step 2 — Durable pruneheight + rolling inwit segments

- **Contract:** `--prune-inwit` / conf (default **off**). When on, after tip
  connect, watermark = `tip.saturating_sub(288 + buffer)` (**height**, not
  a count of inwit blocks). SCHEMA bump: inwit is a rolling segment dir
  (64 MiB sealed files under cold inwit), loc is window-relative,
  below-watermark sentinel. Walker **unlinks** segments wholly `≤ pruneheight`
  (every height in the file, including orphans). Kill-safe: watermark
  advances only after those unlinks. Reopen restores watermark + open
  segments. Leftover genesis-length `inwit.body` without the new layout →
  refuse (OPERATOR: copy-tail then unlink on first pruned open, or refuse
  and tell the operator). Confirm appends only to the live segment. A kept
  height with an orphan sibling keeps **both** inwit payloads.
- **Red:** `prune_watermark_survives_reopen`;
  `unlink_segment_below_watermark_then_pruned`;
  `kept_window_inwit_still_reconstructs`;
  `prune_during_ibd_does_not_grow_historical_inwit_stem`;
  `kept_288_heights_retains_orphan_inwit` — reorg at a kept height; both
  blocks reconstruct; inwit block count **>** the height window.
- **Green:** store meta/sidecar; background unlink (not confirm hot path).
- **Refactor:** no second inwit encoding; no punch path on this table.
- **Verify:** `cargo test -p rbitcoin-store prune_inwit_` ;
  `cargo test -p rbitcoin-query prune_watermark_`
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

- **Contract:** OPERATOR `--prune-inwit`, 288-**height** window (orphan
  inwit can make the block count larger), NETWORK_LIMITED,
  rolling cold inwit (unlink, not punch), cannot reorg through pruneheight,
  archive convert = copy tail + unlink old stem. COMPAT prune row.
  `getblockchaininfo` fields. SCHEMA/sidecar bytes. Partial Esplora JSON.
  Do not copy this file into quality.md until scheduled. Module:
  `pruneInwit` (and buffer if the CLI has one); `coldDataDir` already
  exists. Eval asserts `--prune-inwit`. Runtime label only if tmpfiles /
  `ReadWritePaths` change.
- **Red:** eval assert for `--prune-inwit`.
- **Green:** module + eval + those doc owners.
- **Verify:** `nix build .#checks.x86_64-linux.nixos-module-eval --no-link`;
  grep `NETWORK_LIMITED`, `prune-inwit`, `inwit` segment.
- **Done when:** the [cycle](../how-we-plan.md#the-cycle-red--green--refactor) closed and the slice is committed

## Test budget

Tiny `/tmp` chains (keep window 2–4 **heights** in tests, production 288
heights; include one orphan so inwit block count exceeds the height window).
No mainnet open. One P2P notfound + reconstruct-serve of a kept tip. Unlink
tests on all OS (not Linux-only punch).

## Risks / follow-ups

- Tweaks naive path parent inwit: must not Corrupt on pruned parents.
- mempool.space **frontend** may assume `vin` always present on `GET /tx`;
  personal-node wallets (Electrum history/utxo) do not. Partial JSON is for
  honest objects, not a claim that we are a full electrs archive.
- Core functional prune scripts stay skip until we claim Core `-prune`
  (we do not).
- Thin-inwit (keep prevout fk+vout, drop scripts) would allow fee + vin
  without witness; separate plan if a wallet needs that.
