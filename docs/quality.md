# Quality roadmap (living)

What is strong, what still blocks “industry-leading,” and what is already
closed.

**Last refresh:** 2026-09-13 (schema **21**, Core functional **71** `run` /
**196** `skip`, findings **001–023** fixed, nightly fuzz **20** jobs including
asmap). Previous reaudit: 2026-09-03.

**Three lists only**

| Section | Purpose |
|---------|---------|
| **Open** | Single prioritized backlog — **rank 1 = next** |
| **Won't fix** | Explicitly retired. Do not reopen without a new product decision |
| **Completed** | Finished quality work — do not reopen without new evidence |

North star and working rules are context, not a fourth backlog. Update a
row when work lands. Do not keep a dated LOC / grade snapshot here — those
rotted against Open.

This is not a security audit. Numbers are order-of-magnitude.

**1.0 product gates** (what an operator can count on) live in
[`road-to-1.0.md`](./road-to-1.0.md). Do not copy that list here.

Former `algo-review.md` items **Q-57–Q-60** are Completed. Dual-path and
probe rules are What to protect / [`invariants.md`](./invariants.md). Do
not restore a dated crate table as a second backlog. Close a Q-id by
moving it to Completed in the same PR.

Peer full-node notes (Hornet, satd) live in
[`peer-clients.md`](./peer-clients.md). Ranked later-consideration items
stay there; promote into Open only when scheduling a slice. **Q-30**
(Completed) was the highest-leverage steal (in-tree differential fuzz).

---

## North star: industry-leading full node in Rust

rbitcoin’s thesis is already differentiated: **relational archive** (no UTXO
set), **pure-Rust scripts**, **in-process Electrum/Esplora for wallet backends**,
**Linux map-free IO + optional io_uring**, **reproducible static musl**.
Leadership is not “clone Core’s checklist”; it is **owning that thesis** at a
level peers cannot ignore.

### Pillars (priority)

| # | Pillar | Industry-leading looks like |
|---|--------|-----------------------------|
| 1 | **Correctness under adversarial load** | Consensus-aligned with Core where we claim parity; differential fuzz continuous; findings tracked to **fixed** + regression; no silent confirm/store fallbacks |
| 2 | **Operator trust** | Docs match shipped IO/store; milestone skip impossible to miss; CLI/conf primary; SECURITY contact; honest 0.x; dummy RPC numbers gone or labeled |
| 3 | **Build & release integrity** | Same toolchain in CI and Nix; byte-repro musl; SBOM/audit gates; no floating `stable` |
| 4 | **Contributor velocity** | God-files gone or split by stage; warm suite a few minutes; TDD practiced; first-hour tutorial |
| 5 | **Product surface honesty** | COMPAT accurate; Electrum/Esplora complete for *target* wallets, not explorer bloat |
| 6 | **Observability & ops** | Default INFO shippable; residual env in `docs/env-knobs.md`; signet-first then mainnet with monitoring |
| 7 | **Platform truth** | Linux-first operator binaries; store IO session exists for Darwin (`pool`) and Windows (IOCP). Packaging those OSes is still a later ask |

### Competitive bar

| Peer class | Beat them on | Do not waste effort matching |
|------------|--------------|------------------------------|
| **Bitcoin Core** | Archive model, Electrum-in-process, musl portable binary, pure-Rust script path | Every RPC, GUI, wallet, multi-OS desktop |
| **libbitcoin** | Modern Rust tooling, coverage gates, flake/repro, Electrum/Esplora | C++ cultural norms |
| **Fulcrum / Electrs** | Full validating node + index in one process | Being a pure indexer |
| **Other Rust nodes** | Store design, operator honesty, Linux IO, findings hygiene | Marketing or premature 1.0 |

### Non-goals that still look like “quality”

Not Open, not Completed — see **Won't fix** for retired Q-ids.

- 100% line-coverage theater (gate is **≥90%** LCOV + property-focused tests)
- Rewriting secp256k1 / rust-bitcoin / tokio “to reduce deps”
- Flattening purpose-built io_uring machines to batched `pread` (see [`io-modality.md`](./io-modality.md))
- Core-compatible full RPC surface; graphical block-explorer APIs
- Growing leftover RAM maps to `Vec<Fk>` unless a mainnet miss is shown
  ([`errata.md`](./errata.md))
- Headerless SH extent interiors (uniform 4 KiB page records; ~0.2% density)
- Restoring `rbtc-script-coord-*` (ibd-confirm publishes waves)
- `rbitcoin-bench` in default-members / musl / required CI

---

## Open (priority order)

**One list.** Rank is overall operator + contributor leverage, not a category.
Tags are scan hints only.

**P0 trust/correctness (Q-01–Q-05) stays empty.** Do not reopen without new
evidence (failed Core corpus, new dual path, red required CI, MSRV drift).

Re-checked 2026-09-13: all seven rows still match the tree. None retired.

| Rank | ID | Item | Tag | Done looks like |
|-----:|----|------|-----|-----------------|
| 1 | **Q-41** | Grow Core functional `run` set | test | Inventory `run` covers the wallet-client / P2P / mempool / buried-activation scripts we **claim**. **Today: 71 run / 196 skip (20 rpc-missing, 18 core-log, 68 no-wallet).** COMPAT-done leftovers are `rpc-dialect` (not `rpc-missing`). Next `run` candidate: `p2p_permissions`. `mempool_accept` stays skip (`policy-libre` standardness zoo after type-check). Product-never skips stay skip. Unlabeled PRs stay cargo-only; nightly green. `rpc_createmultisig` `generate(149)` 3-node `sync_blocks`: unanswered `getdata` expires at 10s and is re-asked. |
| 2 | **Q-48** | BIP331 rust-bitcoin package types | interop | Native BIP331 `NetworkMessage` when rust-bitcoin exposes it (**RB-007**). Packages today are RPC `submitpackage` / Esplora `POST /txs/package` only — no private P2P command. Blocked upstream — ranked below unblocked ops work. **After this:** Electrum 1.6 then 1.7 (`protocol_max` bump in the same work) — [`COMPAT.md`](../COMPAT.md) § Protocol versions |
| 3 | **Q-31** | Hermetic tip fixtures | ops | Frozen signet/mainnet tip packs for offline consensus/Electrum regression (no live API). **Fuzz corpora** already merge tiny `signet_block_*.bin` / `mainnet_block_290329.bin` (remined onto regtest). Electrum hermetic packs still Open. |
| 4 | **R-10** | Residual god-files | code | Peel **only** when a higher row needs a seam. Unnamed line-count peels still wait. Do not split `interpreter.rs` opcode `match` or io_uring machines. Production leftover (2026-09-13, tests peeled): `chain` **4.7k**, `tx_relay` **3.9k**, `peer` **3.9k**, `store` **3.8k**, `scripthash` **3.4k**. `electrum/server` production is **1.9k** (`server_tests.rs` holds the journey). **Q-54** may need a seam if a cap rule cannot match a god-file. **Q-61** named extracts are Completed. |
| 5 | **Q-54** | Grow ast-grep rules from `ibd-memory.md` | code | One rule per named cap that is easy to delete: `pending_blocks` 128, `held_bodies` 320, `MAX_SERVE_BLOCKS` 16, `follow_live` vs `max_outbound`. Each rule has `lint/ast-grep/fixtures/{good,bad}/`. Structural rules today are **four** (`detached-tokio-spawn`, `mem-forget-or-leak`, `thread-spawn-dropped`, `crate-root-dropped-pub`) — none of those are cap rules. Peel god-files (**R-10**) only if a rule needs a seam. |
| 6 | **Q-55** | CRAP `--fail-regression` | test | Commit `crap_baseline.json` (`--format json --sort file`) from a green coverage artifact. PRs fail if a function’s CRAP rises. Still no `--fail-above 30`: at ≥90% coverage CRAP equals CC and would force **R-10** peels (`handle_peer_frame` / confirm write / SH pack). Site-local `cognitive_complexity` allows (when justified) do not change that. Clippy workspace policy: [`code-shape.md`](./code-shape.md). |
| 7 | **Q-56** | Miri islands beyond primitives | reliability | `cfg(miri)` tests for FFI-free helpers (scriptnum, pack_ud-style integers) that do not pull secp/store. Never workspace miri. Nightly `miri.yml` is still primitives-only. |

### ID aliases (R-program ↔ catalog)

R-ids were the 2026-08-12 ranked slice. Canonical Open/Completed/Won't-fix
id is in **bold**. Do not start **R-11+** — new work is the next unused
**Q-id (Q-63+)**.

| R-id | Canonical | Where |
|------|-----------|-------|
| R-01–R-06 | **R-01–R-06** | Completed |
| R-07 | **Q-30** | Completed |
| R-08 | **Q-20** | Completed |
| R-09 | **Q-16** | Completed |
| R-10 | **R-10** | Open rank 4 |

Next unused Q-id is **Q-63**.

---

## Won't fix

Retired on purpose. Not a backlog. Not a failure.

| ID | Item | Why |
|----|------|-----|
| **Q-24** | CODEOWNERS / issue templates | No public collaboration process to own. Revisit only if external reviewers are invited |
| **Q-25** | crates.io package metadata | Distribution is `nix build .#rbitcoin-musl`. `repository` is already set |
| **Q-32** | Structured logging option | INFO/DEBUG text is the operator contract. JSON/kv is a second dialect |
| **Q-33** | Published rustdoc site | `cargo doc` locally. No docs.rs until crates.io (Q-25) |
| **Q-35** | Mainnet soak program | Not a program. Run signet first, then mainnet with monitoring. No gated checklist or badge |
| **—** | Darwin notarization / Developer ID | Ad-hoc `codesign -s -` on the macos snapshot. Notarization is still not a product |
| **—** | Leftover maps as `txid → Vec<Fk>` | [`errata.md`](./errata.md): only if a mainnet miss is shown |
| **X-M3** | Esplora process-wide `sh_join` LRU / per-IP / large cache | HTTP is not a session. A tiny LRU still evicts wallets; per-IP is NAT/DoS; a large cache is RSS (join payload × addresses × clients). Sticky joins stay on Electrum TCP (one slot per connection). Esplora keeps one last SH for sequential REST. |
| **—** | Package-level feerate on `accept_package` / `submitpackage` | COMPAT: sequential `accept_tx`; a 0-fee CPFP parent is rejected on its own min-relay. Core `submitpackage` parity is not 1.0. |
| **—** | Esplora `/blocks` reconstruct + chained `scripthash_mempool_stats` | Explorer page cost / dialect. Persist size/weight is a schema ask; graphical explorer APIs are already Won't-fix. |
| **—** | Retired algo-review micro-opts | BDZ page fill, `HashHead::bulk_fill_empty` RAM, SH `insert_many` N², INV O(peers×mempool), mempool `find_free_slot` O(cap), `evict_nonfinal` O(n²), BQ `index.iter().find`, `BlockCache` prefix O(chain), GBT `depends` scan, log-macro eval when disabled, `api_log` mutex, bit-by-bit `count_ones`, `U64IdentityHasher` clustering, `last_push_data` PUSHDATA4, `--api-log` unbounded JSONL. Not a second backlog. Reopen a named Q-id only with a mainnet profile that names the cost. |
| **—** | Headerless SH extent interior pages | Extent is a span-read of the existing 4 KiB delta-page record. Interiors keep `ver`/`n_fks`/`next` so one decoder serves leftovers, tails, and last-page append. Full-page payload is ~0.2% and a schema bump |
| **—** | Restore `rbtc-script-coord-*` | `ibd-confirm` publishes waves, polls lock-free completion, feeds `scriptq` when steal is empty. Steal workers unpark the publisher. Do not add coordinator threads to keep the pool fed |
| **—** | Flatten purpose-built io_uring machines | [`io-modality.md`](./io-modality.md): fix the machine; do not replace it with batched `pread`/`pwrite` without an explicit ask |
| **—** | Process pin FIFO / CreateResidency / ContigPark / archive sticky | Pins are plan/batch only. IBD confirm is body-queue wire → lookup → load. [`concurrency.md`](./concurrency.md), [`invariants.md`](./invariants.md) |
| **—** | `rbitcoin-bench` default-member / musl / required CI | Optional crate, host A/B against a live store. Not a packaging or coverage gate (`coverage.sh` `--exclude` + IGNORE) |
| **—** | `cargo miri test --workspace` | io_uring, tokio, secp256k1-sys. Too heavy / cannot go green. Primitives only (**Q-53**); extra islands are **Q-56**. |
| **—** | `cargo crap --fail-above --threshold 30` | At ≥90% line coverage CRAP **equals CC**. `handle_peer_frame` / confirm write / SH pack would force **R-10** peels. Use **Q-55** regression instead. |
| **—** | ast-grep as a second clippy for style | Structural rules catch RSS/task-leak *shapes*. Clippy policy is [`code-shape.md`](./code-shape.md) (no workspace allow list; leftover lints are site-local). |

---

## Completed

**Toward 0.7** (keep the contract). Older closures live in
[`CHANGELOG.md`](../CHANGELOG.md). Do not reopen without new evidence.

| ID | Item | Resolution |
|----|------|------------|
| **Q-38** | Slim live P2P in default CI | Hop serve, 8-block dual live seeders, post-IBD tip follow, getheaders gap fill, and product `run_p2p --connect` run in `cargo test` / coverage. 48-block dual-seeder, 20-block combo, and 4-node mesh deleted. `scripts/integration.sh` deleted. |
| **Q-62** | IBD load `txout.body` read bandwidth | Need-aware Outs extend: first 4 KiB peek is complete when every `need_vouts` `skip_at`s in-page (empty need still walks all outs). Overlapping body peeks in one uring wave share one OS-page SQE (cap two pages). `ibd: perf` `cold_range` `extend=` / `sqe=`. Random 4 KiB parent faults, spent page-RMW, and leftover TipOnly stay as designed (no coins cache, no persist-in-flight). |
| **Q-61** | 0.6.0 readability (code shape) | Display-hash owner (`display_hash_hex` / `parse_display_hash32`); CLI `CliAccum` / `apply_kv`; RPC `METHOD_LIST` catalog so `help` / `getrpcinfo` list every dispatched method; Electrum/Esplora `sh_at_view`; `PeerFollowState` + `PendingSendCmpct` + shared mempool GetData; `CatchUp` + named `run_p2p` phases + shared hub-tip bridge. IBD confirm events drain through `apply_confirm_events`; Headers apply is named stages. `ChainHub` holds `HeldBodies` under one `RwLock` (cap 320); `Invalidated` / `HeaderTips` / `MiningKnobs` are named types. Query SH write-behind is `ShWriteBehind`; `IndexMode` names archive-spend / SH-enqueue products; `TxTable::probe_body_match_fk` is the body-txid head probe. Mempool `scan_conflicts_and_parents` / `evict_worst_chunks` / hub `admit_staged`. One-shot confirm load is stamp + load_from_plan; pin stages named; `ScriptVerifyFlags` on script jobs. Confirm reject class (`SoftMerkle` / `SoftRetarget` / `BadPrev` / `Permanent`) is set at the sender. Stretch closed. Owner: [`code-shape.md`](./code-shape.md). |
| **Q-57** | Store publish / Class C flush / sidecar | `published_meta` Acquire. `ArrayTable` / `StrongTxTable` `flush_dirty` packed dirty-epoch (`0`=clean; snapshot under read; CAS `e0→0`; wrap of `u64::MAX` stays dirty). fuse8 `decode_body` refuses fingerprint arrays shorter than `hash_of_hash` geometry. Leftover fuse8 v1 refuses open. `for_each_spender_create` hops ≤ `spenders.count()`. Seal/install of `meta` / `.mphf` / SH `.idx` is tmp + `sync_all` then rename. `list_runs` is a scan (open-time SH `key_len` and leftover-run count do not delete). |
| **Q-58** | Mempool persist order + eviction | Admit `persist_all` writes body then LIVE slots then meta. Compact writes `tx.body.tmp`/`slots.tmp`, `sync_all` both, rename body then slots, then persist **meta only** (open finishes a leftover `slots.tmp`). Known-parent OOB vout is `MissingPrevout`. `evict_to_budget` no-op break. `evict_worst_chunk_once` uses `remove_txid_tree` so a parent-only worst chunk cannot leave a child. |
| **Q-59** | RPC / CLI honesty | `submitblock` all-networks via `ChainHub`. `--minrelaytxfee` garbage/negatives fail start. `getnetworkhashps` labeled dummy 2-work-per-block. `gettxout include_mempool` hides mempool-spent confirmed outs. RPC-submit `maxfeerate` (default 0.10 BTC/kvB, `0` unlimited) / `maxburnamount` (default 0) on sendraw / testmempoolaccept / submitpackage; P2P `accept_tx` uncapped. JSON-RPC array batch `len > --rpcworkqueue` is HTTP 500 when the permit is set. `getmininginfo.blockmintxfee` is `sat_btc_json` BTC/kvB. |
| **Q-60** | P2P caps + compact reconstruction | Compact prefill monotonic; held FIFO; getdata 10s expire; outbound asmap/prefix diversity. AddrMan 8192 on learned/`peers` load/`addpeeraddress` (evict incompat → failed → oldest new); `--connect`/DNS `add` may exceed when only tried remain; `merge_from` trims to 8192. `announced_wtx` / `from_this_peer` insertion-order FIFO-roll at 50k. `cmpct_fills` decrement on fail/expire/unregister (Accepted `clear_cmpct_fill`). |
| **Q-30** | Continuous differential fuzz | Nightly `fuzz.yml` (**20** jobs, not a required PR check): BIP324 parser (`v2_contents`) + live Core session (`v2_session`), header/block `submitblock` (height-1 / spend / fork / N-reorg / BIP68 CSV-age), compact reconstruct vs Core `getblocktxn`, compact reorg via `drain_pending`, script-mutating vs Core, ASan wire parsers, and ASan asmap bytecode (`asmap`). Crashes → `docs/external_findings/` + named regression. JSON corpora stay static. Job list: [`TESTING.md`](../TESTING.md). |
| **—** | Compact reconstruct merkle-check | A unique short-id (or `blocktxn`) fill is not a block until txs match the compact header merkle (BIP152 `FinishBlock`). Empty missing → `getdata`, not `accept_branch`. A merkle/`bad-txnmrklroot` that still reaches `accept_branch` is not cached `BLOCK_FAILED`. |
| **—** | Electrum 1.4 leftover + unsubscribe | `blockchain.scripthash.unsubscribe` returns whether the connection was watching (frees the 1000-sub cap). `get_history` unconfirmed rows include `fee`; confirmed rows omit it. `listunspent` mempool height is `-1` when a parent is still in the mempool. Confirmed methods skip hub txs already on the tip. |
| **—** | `--sptweaks-dust` | Serve-time Electrum tweaks floor (default **1000**; `0` = all; **546** matches Cake electrs). Index unchanged. |
| **—** | API envelope pins | JSON-RPC HTTP/Basic junk, RPC param types / unknown named keys, Electrum param types / asof leftover / sub cap, Esplora junk paths / asof gating / POST bodies. |
| **—** | 2026-09 crate simplification | Crate-root `pub` graph (X-04, `crate-root-dropped-pub` rule); dead meters then instance `ConfirmStats`; `HeadScale` / `StoreLayout::tiny`; Tiny `testutil`; per-crate dead paths; fuzz in `fuzz/`; CLI/conf share `apply_kv`; leftover index refuse; one assemble path; one Class A planner. Clippy workspace allow list is empty; leftover lints are `#[allow]` on the item with a reason ([`code-shape.md`](./code-shape.md)). |
| **—** | IBD main-loop / leftover / tip-accept (2026-09-03) | Assign ≤50 ms; header locator poll ≤500 ms; stall hygiene ≤1 s. `clear_leftover_miss` no longer wipes `diag=1`. P2P `tx` / Esplora `POST /tx` `spawn_blocking`. Tip reconstruct / RPC generate do not take `connect_lock` on a tokio worker. Higher-height less-work fork is `register_explore` only. Empty headers at a drained most-work path is EOF; `seed_work_path_from_store` only when `ordered` is empty. |
| **023** | Tapscript initial stack 1000/520 | After OP_SUCCESS scan, tapscript rejects `stack.len() > 1000` and elements `> 520`, matching Core `ExecuteWitnessScript`. |
| **Q-51** | ast-grep structural lints | `sgconfig.yml` + `lint/ast-grep/` (`detached-tokio-spawn`, `mem-forget-or-leak`, `thread-spawn-dropped`, `crate-root-dropped-pub`) + `scripts/ast-grep.sh`. Required CI job `ast-grep`. Named-cap rules: **Q-54**. |

### Earlier (one line)

On-disk version is **21** ([`SCHEMA.md`](../SCHEMA.md) / [`SCHEMA_HISTORY.md`](../SCHEMA_HISTORY.md);
0.x may still bump). Confirm park/unpark steal, ibd-confirm waves, process-wide
head drain, BIP141 nonce skip. Wallet-client last-slot SH join + optional
`rbitcoin-bench`. Core functional first green was 9 scripts; **71** `run` now
(remaining growth is **Q-41**).

| ID | Item | Resolution |
|----|------|------------|
| **Q-52** | CRAP report on coverage LCOV | `scripts/coverage-crap.sh` after the ≥90% gate; `coverage/crap.json`. Regression gate: **Q-55**. |
| **Q-53** | Miri on primitives | `scripts/miri.sh` → `cargo miri test -p rbitcoin-primitives`. Nightly `miri.yml`. Islands: **Q-56**. |
| **Q-50** | Perf meter residual coverage | Named write/lookup/load inventory + explicit `other=`. Fat `other=` later is confirm-perf, not a meter program |
| **Q-36** | Perf log diet | Default INFO is `ibd: progress`. `ibd: perf` / `ibd: sizes` / `ibd: perf_dbg` at DEBUG |
| **Q-34** | First-hour tutorial | [`OPERATOR.md`](../OPERATOR.md) § First hour (regtest) |
| **Q-49** | v2-only peer discovery | `x809.<seed>` first, then unfiltered; `addr`/`addrv2` requires `P2P_V2` |
| **Q-47** | Honest `getblockchaininfo` disk / progress | Store file walk + `blocks/headers` (not dummy 0 / 0.5) |
| **Q-37** | Warm default suite ≤3 min | Required CI `test` **~85 s** (2026-08-17, ubuntu-24.04). Stretch &lt;2 min met on CI-class. [`TESTING.md`](../TESTING.md) |
| **—** | Docs map + tests assert behavior | `docs/README.md`; no `include_str!` of production `.rs` / CONTRIBUTING |
| **Q-15 / Q-42–Q-46** | CLI, inbound config, RPC honesty, Libre-only, IO aliases | 2026-08-16 cruft program |
| **R-01–R-06** | Mempool snapshot, `script_pool`, remine pads, TxGraph cache, llvm-cov pin, tip-follow store integrity | 2026-08-12 |
| **Q-16 / Q-20 / Q-23** | Residual env, `cargo deny` CI, optional musl artifact | `env-knobs.md`; required `deny`; musl zip is GitHub Release only |
| **—** | Darwin / Windows operator snapshots | GitHub Release (`release.yml`). PR `windows` / `macos` smoke store IO + `--smoke` |

Q-01–Q-14, findings 001–022, CI split, map-free README: [`CHANGELOG.md`](../CHANGELOG.md).

---

## Working the list

| Do | Do not |
|----|--------|
| Close work by **moving the Open row into Completed** in the same edit as the landing change | Leave `Status: fixed` in Open, or start a second table |
| New item: next unused **Q-id (Q-63+)** inserted at an explicit rank | Fill historical gaps (Q-06–Q-09, Q-17–Q-19, Q-26–Q-29) or start **R-11+** |
| Retire a row to **Won't fix** when the product will not do it | Leave dead Open rows “for completeness” |
| God-file peels when a higher Open row needs a seam (**R-10**) | Split `interpreter.rs` opcode `match` / io_uring machines / MPHF to beat a line count |
| Suite: no new remine-100 / default test **&gt;2 s** without justification ([TESTING.md](../TESTING.md)) | Time the full workspace as a planning spike |
| Differentials / crashes → `docs/external_findings/` + named regression | Soft dual paths on confirm identity / denserels / Class A load |

---

## What to protect

- Distinct product thesis (archive + pure Rust scripts + in-process wallet APIs).
- Small dependency graph; no `libbitcoinconsensus`.
- Operator honesty (experimental, milestone, Linux-first, honest MSRV).
- Written concurrency model (roles, HWM, one Class A appender; ibd-confirm
  publishes script waves — no coordinator threads).
- Portable static musl + crane + repro notes.
- Warnings-as-errors; Red → Green → Refactor (`docs/how-we-plan.md`).
- SCHEMA / SCHEMA_HISTORY / crash-recovery / COMPAT at 0.x. Current bytes
  and refuse messages: [`SCHEMA.md`](../SCHEMA.md). Soft-migrate durable
  side formats; no silent wipes.
- External findings hygiene + Core corpora without allowlist.
- Confirm dual-path kill + live P2P IBD / hop-serve / tip-follow in default `cargo test` / coverage.
- Tests assert shipped behavior, not repo text.
- Sealed fuse8 fingerprints stay RAM; BDZ `g` is FdOnly.
- Optional `sp_tweaks` leftover regenerate is not a Class A wipe.
- One Class A planner (`archive_class_a_from_wire`); TxApply→dummy `Block`
  only in `rbitcoin_query::testutil`.
- Instance `ConfirmStats` / session IO stats; no process-global confirm
  meters and no TLS `test_take_*` probes.
- `HeadScale` is open-time `StoreLayout`; production default Mainnet.
- CLI and conf share `apply_kv`. Fuzz harness lives in `fuzz/`.

---

## Consumers

| Audience | Read |
|----------|------|
| Next quality slice | **Open**, rank 1 (**Q-41** Core functional `run` set). Residual peels: **R-10**. Next unused Q-id is **Q-63** |
| Peer full nodes | [`peer-clients.md`](./peer-clients.md) — Hornet / satd notes; not a fourth backlog |
| Release engineering | **Q-20**, **Q-21**, **Q-23** (completed) |
| Security / adversarial | Protect Q-01–Q-02; **Q-30** completed |
| Docs / README | Map is done; first hour is OPERATOR.md (**Q-34** closed) |
| “Are we leading yet?” | North star + Open |

---

*Living document. Prefer updating this file over dated audit copies.
Reaudit after a multi-commit quality program or when Open claims would rot.
Do not restore a LOC snapshot or grade board as a fourth list.*
