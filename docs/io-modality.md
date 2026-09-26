# Store IO modality matrix

**Source of truth** for bulk `RBITCOIN_IO` vs table transport (fd + tiered RAM).
**Phase 6 complete:** workspace has **zero `memmap2` / `MmapMut`**.
[`TableFile`](../crates/rbitcoin-store/src/file.rs) stays **FdOnly**. The mmap
exceptions are **read-only immutable** sealed `.fuse8` fingerprint arrays
([`fuse8_filter.rs`](../crates/rbitcoin-store/src/fuse8_filter.rs)) and SH
BDZ3 occupancy bitvectors (prefix of `NN.mphf` through occ, not tags). Unix
`mmap` `PROT_READ`/`MAP_SHARED`, Windows `MapViewOfFile` `PAGE_READONLY`. Packed
MPHF `g` stays FdOnly 4 KiB: a mapped miss is one synchronous fault on the lookup
thread, while `KIND_MPHF_G` keeps 128 pages in flight on the completion
session. `strong_tx` and mempool stay process `Vec`.

**Map vs FdOnly:** map when the bytes are immutable after write, touched as
a few random bytes per key, and must stay resident for the hot path (sealed
`.fuse8`: every unfinished key probes every sealed segment; SH BDZ3 `occ`:
1.23 bits/key, rank on every compact index). Keep FdOnly +
completion session when the file is larger than the residency budget or is
read as page batches (`txid.body` / `txout` / `spent` / `create.loc`, packed
BDZ `g`, SH tags/`.val`).

Related: [`env-knobs.md`](./env-knobs.md), [`concurrency.md`](./concurrency.md),
[`crash-recovery.md`](./crash-recovery.md), [`architecture.md`](./architecture.md).

---

## Two independent layers

| Layer | Controlled by | Values | Purpose |
|-------|---------------|--------|---------|
| **Bulk batch** | `RBITCOIN_IO` only | `uring` \| `pool` \| `iocp` \| `pread` | Multi-op **completion session** on file handles (`txout` pin/outs, `seqsigwit` reconstruct, spend meta/ann on `spent`, Class C bulk) |
| **Table transport** | [`TableFile`](../crates/rbitcoin-store/src/file.rs) | **FdOnly always** | All payload via pread/pwrite; fallocate grow; no process maps. Sealed `.fuse8` and SH BDZ3 occ are sidecar maps, not TableFile |

**`RBITCOIN_IO` selects the completion-session backend** (not per-path).
Unknown tokens (including deleted `mmap`) fall through to the platform default.

| Token | Backend |
|-------|---------|
| `uring` / `io_uring` | Linux `io_uring`. On Windows the token opens **IOCP** |
| `pool` | Worker-pool completion ring (Darwin **default**; Linux CI pin) |
| `iocp` | Windows IOCP (Windows **default**) |
| `pread` / `fd` / `libc` / `pwrite` | Disable session; libc positional IO (+ workers for reads) |

**Defaults:** Linux `io_uring` if the ring opens, else pool. Darwin **pool**.
Windows **IOCP**. Windows IoRing is not supported.

**Windows table handles** are `FILE_FLAG_OVERLAPPED` so IOCP can bind.
Create, open, header/trailer, and grow use positional `IoHandle`
pread/pwrite and `SetFileInformationByHandle(FileEndOfFileInfo)`. Do
**not** mix those handles with std `Read`/`Write`/`Seek` — `WriteFile`
with a NULL `OVERLAPPED` is os error 87 (`ERROR_INVALID_PARAMETER`).
Positional xfer sets the low bit of `OVERLAPPED.hEvent` so the packet is
**not** queued to the process IOCP (harvest `Box::from_raw`s only session
heap OVERLAPPEDs).

**kqueue is not a regular-file backend.** Darwin files report ready immediately;
`read` still blocks. POSIX AIO (`EVFILT_AIO`) and `dispatch_io` are also
thread pools (`kern.aiomax` default 16) — same class as `pool`, not an
SQ/CQ. The pool session is the honest Darwin completion ring. Pool
**workers** are process-shared; each TLS session keeps its own CQE queue.

**TLS session:** one completion session per OS thread (`with_thread_local`).
Nested calls panic. Harvest tracks pending `(kind, epoch, slot)`. A CQE that
is unmatched, duplicate, from a prior epoch, leftover after `drain_all`, or
from CQ overflow is `Corrupt` — not a completion and not a TipOnly miss.
`begin_batch` drains leftover SQEs before bumping epoch and **returns `Err`**
if the session is poisoned or leftover cannot drain (probe / identity / BDZ
g-pages / rel preads share the ring). Held fill on that ring fails closed on that
error (no libc fallback on a dirty ring). Head-resolve `create.loc` is **not**
on the probe ring: one FdOnly batch after identity (standalone bulk; live shorts
libc-complete; no probe recover credit). `range_batch_ctx` on a still-held
session keeps poison fail-closed and live-ring libc-complete. Undrained / unexpected /
wait-timeout **poisons** the session and drops the TLS ring so the next wave
opens a new one. `submit_and_wait_one` shares the drain budget (slow-log at
5 s, poison at the hard cap). Linux wait is Ready only when the CQ has a
visible event (`io_uring_enter` returns SQEs submitted, not CQE count); an
empty CQ is TimedOut. Live harvest loops wait, then retry; one empty harvest
is not batch death. `drain_all` on every TLS session (Linux, pool,
IOCP) waits while CQEs keep arriving; after 5 s it logs `store: io_uring
drain slow`. Zero completions for `RBITCOIN_URING_DRAIN_HARD_SECS` (default
120) abort **explicit** `drain_all` / `DrainOnDrop` (buffers still pinned).
Session `Drop` waits the same budget then poisons without aborting so a
stalled device cannot `abort` from a destructor. There is **no** runtime
switch to `pread`. Drain before SQE buffers drop (spend annotate
`DrainOnDrop`). Per-op short/errno on a live session still libc-completes
that op; libc fail is `StoreError::io`. A live unpoisoned session that fails
the bulk batch after the wait budget (submit / leftover) is the same
libc-complete, not a recover credit. `RBITCOIN_IO=pread` is the only
whole-batch pread fallback (session unavailable also falls back; operator
restart after a drain abort).

IBD write / lookup / load / scripts and post-IBD tip connect: a session fault
after a successful drain recovers once per 1000-height window (credit CAS;
**no** in-process Class C repair — leftover strong is the open-repair case).
The next wave opens a new TLS ring. A second stall in that window aborts and
names `RBITCOIN_IO=pread`.

### Do not flatten custom machines

Machines (spend-annotate RMW, fused head-resolve, pipelined bulk fill) stay
**multi-stage**. Pool/IOCP are session backends — they do not flatten those
loops to one-shot `pread_batch`.

**Do not** replace a purpose-built / multi-stage **io_uring machine** with
batched `pread`/`pwrite` / one-shot `pread_batch`/`pwrite_batch` **without
explicit permission from the user**.

| OK | Not OK without permission |
|----|---------------------------|
| Fix bugs inside the existing machine | Delete/retire a custom machine and call bulk batch helpers |
| Thread new flags through the same SQE path | “Simplify” to serial pread + one big submit |
| Fall back to pread when uring is unavailable | Rewrite a machine away “because batch is enough” |

If a change seems to require collapsing a machine, **stop and ask**.

---

## RAM tiers (L0 / L1 / L2)

| Tier | Where hot bytes live | Sync |
|------|----------------------|------|
| **L0** | Kernel page cache via pread/pwrite; process holds staging only | Payload then HWM publish; `sync_data` on flush barriers |
| **L1** | 4 KiB head pages / 3–4 KiB SH chunks (working-set caches) | Write-back dirty page/chunk with one pwrite |
| **L2** | Compact Class C (`confirmed`, `header_txs_*`, `strong_tx`) full `Vec` in process | **Write-behind:** RAM mutate during commit; complete-or-fail body image on `flush_class_c_tip` **before** body-queue dequeue |

**Never L2:** `txout` / `seqsigwit` / `spent`, full `tx.head` / `*.idx`.
`RBITCOIN_CLASS_C_INRAM_MAX_MB` (default 256) caps **`confirmed`** and
**`header_txs_*` only**. `strong_tx` stays L2 (1 bit/fk). Create height is a
RAM fence (~15 MiB at 1M blocks), not a file.

---

## Current matrix (`RBITCOIN_IO=uring`)

### Bulk batch (env)

| Path | Env | Syscalls |
|------|-----|----------|
| Pin outs / body pipeline | `RBITCOIN_IO` | uring/pread on **`txout.body` FD** (Full also zips `seqsigwit`) |
| Head-resolve identity | `RBITCOIN_IO` | uring/pread on **`txid.body`** (not a packed body prefix) |
| Spend-meta 8 B peeks | `RBITCOIN_IO` | uring/pread on **`spent.body` FD** |
| Spend pure-write annotate | `RBITCOIN_IO` | uring/pwrite or pwrite on **`spent.body` FD** |
| Class C create-height | (RAM fence) | no IO |
| Class A body/idx **linear append** | always | **pwrite** (three stems + three idx) |
| SH unsorted collect (Class A) | libc `pread` | **Two** sequential coalesced **16 MiB** `txout.body` scans (`sh_extract_workers`): static create-fk spans (no steal); per-worker identity maps spill-largest as `SHKSP01` under `keys/NN/` (one spill writer, 1-slot queue; tmp+rename, no `sync_all`; first-fk delta singles); merge folds those files into one map, one walk to `scripthash.head/NN` + `multi/NN.fuse8`; leftover keys shapes refuse. Then fuse-hit `SHPST01` post spills under `post/NN/` (`80n+8f` estimate, same 1.5 GiB worker cap and 1-slot writer; tmp+rename, no `sync_all`). Not TLS uring: one large positional read per span, not a completion machine. Writes are libc `pwrite` / tmp+rename. Pass 2 does not Fd-index the main MPHF; pack folds one shard's spills then `slot_for_key16` once per unique multi key. Packed 2-bit `g` is ~8 MiB per pack worker, not a 12 GiB `.val` page-cache set. |
| SH Electrum/Esplora join | `RBITCOIN_IO` | Query-thread session: waved `idx_body_pipeline` on **`txout.body`**, optional page-grouped **`txid.body`** (history: creates+spenders; listunspent: unspent creates; balance/`/address` stats: none), `get_spender_meta_at_abs_batch` on **`spent.body`**. Megakey **extent**: one span pread of `extent_n` pages, then linked 4 KiB tail. Schema-18 mode 10 leftovers **refuse** on open. Does **not** share a confirm TLS ring. |
| Block index build (filters + tweaks) | `RBITCOIN_IO` | `read_index_window`: one held TLS session per window, four stages (`create.loc` / `input.loc` / `seqsigwit.loc` windows → block `txout` / `input.body` / `txid.body` spans → parent `create.loc` + P2TR-output `seqsigwit.body` + parent `txid.body` → parent `txout`), offsets from RAM checkpoints so nothing nests. Parents read once per window. Short/errno on a live session libc-completes; no session → serial pread through the same stages. Writes are not on the ring: one filter put and one tweak put per window. |
| SP-tweak hole serve | `RBITCOIN_IO` | `tweaks_for_height` for heights the index has not sealed: idx ranges **before** TLS ring; uring/pread `txout.body`, then `seqsigwit` + parent `txout` for P2TR only. |

Default: Linux uring if the ring opens else pool; Darwin pool; Windows
IOCP. Ring depth **128** (merge may grow). `RBITCOIN_IO=pread` forces libc.

### Table transport (all fd)

| Object | Tier | Notes |
|--------|------|--------|
| **`txout.body`** | L0 | Hot outs (pin / SH / Electrum tweaks); pread/pwrite/uring |
| **`seqsigwit.body`** | L0 | Cold ins+witness; reconstruct / getdata only |
| **`spent.body`** | L0 | 8 B×n_out sole-spender; annotate RMW |
| **`create.loc` / `seqsigwit.loc`** | L0 | FdOnly 2 B/create (hot) / u16 (cold); leftover `spent.off` unlinked. `create.loc` leftover stamp: batched window preads, sum/read through max fk in-window, running-sum + SIMD deinterleave (no loc L2) |
| **`tx.head` segments** | L0+L1 | Open OA: 4 KiB page-coalesced RMW. Sealed: mmap `.fuse8` (heap `fuse8=0`); packed BDZ `g` FdOnly 4 KiB page stream (`KIND_MPHF_G`); MPHF output is `rel−1` |
| Header hash head | L0+L1 | 128-slot (~3 KiB) chunk cache |
| Hash multi-list (`.mlt`) | L0 | Linear append |
| **`scripthash.head` / body** | L0+L1 / idx in process | Sealed MPHF main: mapped occ + BDZ `g` FdOnly + tag/val pread, **no fuse**. Ingest/OA: 4 KiB chunk cache. Sealed ovf L0 SHSR: idx + mapped `.fuse8`. L1 ovf: mapped fuse + mapped occ + FdOnly `g`. Body slabs L0 |
| **Spenders** | L0 | Linear append |
| `confirmed` / `header_txs_*` / `strong_tx` | **L2** | InRam write-behind; barrier = `Store::flush_class_c_tip` |
| Create-height fence | RAM | Built from confirmed + header_txs; no `tx_height.body` |
| Mempool (`{datadir}/mempool/*`) | L2 sidecar | Private; **not** Class A |

### Hybrid paths (easy to misread)

| Path | Table part | Fd/uring bulk part |
|------|------------|---------------------|
| Pin outs | FdOnly `create.loc` ranges (batched window preads; sum/read only through max fk in-window) | uring/pread `txout` bytes (starting OS page; full span if need is likely to spill) |
| Head resolve stream | FdOnly **page-batched** head probe on the held session; **one** FdOnly loc batch after identity (standalone bulk, not on the probe ring) | uring/pread `txid.body` identity |
| IBD **getdata serve** reconstruct | FdOnly `create.loc` / `seqsigwit.loc` ranges for a contiguous `header_txs` run | libc span pread of `txout.body` + `seqsigwit.body` in parallel (not confirm `idx_body_pipeline`) |

---

## Head insert locality

Do not demap heads without an operator-host measurement (musl static).
A host A/B showed io_uring head inserts about 5× slower on head ms/blk than
a mapped insert. Prefer page-coalesced pread → mutate → pwrite over per-slot
uring. Segmented heads do not remove that page-locality constraint.

---

## Settled IO shape

Phase 6 is done: the workspace has zero `memmap2` / `MmapMut` (top of this
file). Multi-GiB random tables stay FdOnly. Small Class C and the mempool
stay explicit process buffers. A host A/B comes before a demap. Agent
correctness tests under `/tmp` are required; perf ship/fail is host-only.

---

## Host benchmarks (operator; musl static)

### Rules

- Run on a **real host** with a **local filesystem** datadir (not agent 9p workspace).
- Use the **portable static musl** binary — same as release (`docs/reproducible-builds.md`).
- **Do not** use `nix-shell --run 'cargo build -p rbitcoin-node --release'` for IBD
  benches (Nix-store glibc dynamic link).

### Build musl `rbitcoin-node`

```bash
# Repo root; Nix flakes enabled
nix build .#rbitcoin-musl --out-link result
mkdir -p target/release
install -m 755 result/bin/rbitcoin-node result/bin/rbitcoin-cli target/release/
file target/release/rbitcoin-node   # must say "statically linked"
# or: ./scripts/repro-build.sh
```

### IBD / tip-rate window (primary live gate)

```bash
export RBITCOIN_IO=uring   # or pread for second arm
./target/release/rbitcoin-node --datadir /path/to/local/datadir …flags…

# Capture steady-state minutes:
grep 'ibd: perf' host.log
grep 'ibd: perf_dbg' host.log    # head=, plan_batch, class_a head insert, pin
grep 'ibd: sizes' host.log       # rss= anon= file=
```

**A/B:** same host, same network/milestone/height band when possible; baseline
SHA vs candidate SHA; compare tip rate, head ms/blk, RssFile. **Fail ship** on
5×-class head regression or agreed **>+20%** head ms/blk without tip-rate win.

There is no separate store microbench binary (`rbitcoin-store-bench` was
removed; default graph is product + suite). Head-insert A/B is the live
`ibd: perf` / `ibd: perf_dbg` window above.

`TableFile` has no maps (`memmap2` not in the workspace). There is no
`RBITCOIN_TX_HEAD_ACCESS` hatch. Tables are fd pread/pwrite + fallocate.
Sealed `.fuse8` sidecars and SH BDZ3 occupancy prefixes are the mmap
exceptions (read-only, not `MmapMut`). Packed `g` stays FdOnly. Class C is
L2 write-behind (`flush_class_c_tip`
before BQ dequeue) — **not** mapped (`strong_tx` write-behind is tip-last).
Mempool schema 3 is InRam Vecs + `pwrite`.

Live head insert is page-coalesced pread → mutate → pwrite (not per-slot uring).
Head resolve batches one pread per distinct probe page, then one loc batch
after identity (not on the probe ring). Node start logs `io=`,
not `tx_head_access=`.
