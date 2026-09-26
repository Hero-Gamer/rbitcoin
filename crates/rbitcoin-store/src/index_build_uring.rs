//! Completion-session reader for the post-IBD block index builder (BIP158
//! basic filters and BIP-352 tweaks).
//!
//! One call reads a **window** of consecutive heights through four stages on
//! one held session, thousands of reads in flight per stage:
//!
//! 1. `create.loc` windows for every create in the window, each block's
//!    `input.loc` span, and (tweak heights) `seqsigwit.loc` windows.
//! 2. Each block's `txout.body` span and `input.body` span; each block's
//!    `txid.body` span when any height wants tweaks.
//! 3. `create.loc` windows of parents outside the window; `seqsigwit.body`
//!    of tweak-height txs with a P2TR output; parent txids for tweaks.
//! 4. Parent `txout.body` records.
//!
//! Offsets come from RAM checkpoints, so no stage opens another session.
//! Parents are read once per window however many blocks spend them. A per-op
//! short read or errno on a live session completes with a libc pread; with no
//! session the same stages run as serial preads.

use crate::error::StoreError;
use crate::input::{decode_edges_span, InputEdge};
use crate::io_handle::IoHandle;
use crate::sp_tweaks_uring::LoadedTweakTx;
use crate::tx_table::{
    apply_input_edges, decode_packed_tx_outs_with_spender_rels_secret, decode_seqsigwit_secret,
    OutputRecord, TxTable,
};
use crate::txid_body::{TxidBody, TXID_ENTRY_LEN};
use crate::uring_session::{self, UringSession};
use crate::U64Map;
use rbitcoin_primitives::{Fk, Height};
use std::path::Path;

/// Ring depth for the builder's session.
const ENTRIES: u32 = 256;

/// One height to read: its confirmed header and contiguous creates.
#[derive(Clone, Copy, Debug)]
pub struct IndexHeight {
    pub height: Height,
    pub header_fk: Fk,
    pub first: Fk,
    pub n: u32,
    /// Also read what tweaks need (txids, `seqsigwit` of P2TR-output txs).
    pub tweaks: bool,
}

/// One block's creates, in block order, with their input edges.
pub struct IndexBlock {
    pub height: Height,
    pub header_fk: Fk,
    /// `inputs` is `Some` only for tweak-height txs with a P2TR output.
    pub txs: Vec<LoadedTweakTx>,
    pub edges: Vec<Vec<InputEdge>>,
}

/// A window of blocks plus the outputs of every parent they spend that is
/// not itself in the window (`txid` is zero unless the window wants tweaks).
pub struct IndexWindow {
    pub blocks: Vec<IndexBlock>,
    pub parents: U64Map<([u8; 32], Vec<OutputRecord>)>,
    /// Create fk → `(block, tx)` for creates in `blocks`.
    in_window: U64Map<(usize, usize)>,
}

impl IndexWindow {
    /// Outputs of create `fk`, from this window's blocks or parents.
    pub fn outs(&self, fk: Fk) -> Option<&[OutputRecord]> {
        match self.in_window.get(&fk.0) {
            Some(&(bi, ti)) => Some(&self.blocks[bi].txs[ti].outs),
            None => Some(&self.parents.get(&fk.0)?.1),
        }
    }

    /// Output `vout` of create `fk`, from this window's blocks or parents.
    pub fn prevout(&self, fk: Fk, vout: u32) -> Option<&OutputRecord> {
        self.outs(fk)?.get(vout as usize)
    }

    /// Txid of create `fk` (zero when the window did not read txids).
    pub fn txid(&self, fk: Fk) -> Option<[u8; 32]> {
        match self.in_window.get(&fk.0) {
            Some(&(bi, ti)) => Some(self.blocks[bi].txs[ti].rec.txid),
            None => self.parents.get(&fk.0).map(|p| p.0),
        }
    }
}

fn is_p2tr(spk: &[u8]) -> bool {
    spk.len() == 34 && spk[0] == 0x51 && spk[1] == 0x20
}

struct ReadJob<'p> {
    fd: IoHandle,
    path: &'p Path,
    off: u64,
    buf: Vec<u8>,
}

impl<'p> ReadJob<'p> {
    fn new((fd, path): (IoHandle, &'p Path), off: u64, len: u64) -> Self {
        Self {
            fd,
            path,
            off,
            buf: vec![0u8; len as usize],
        }
    }
}

/// Complete `buf[done..]` with libc preads.
fn libc_complete(job: &mut ReadJob<'_>, mut done: usize) -> Result<(), StoreError> {
    while done < job.buf.len() {
        let rc = job.fd.pread(job.off + done as u64, &mut job.buf[done..]);
        if rc <= 0 {
            let err = if rc == 0 {
                std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "index build pread short")
            } else {
                std::io::Error::from_raw_os_error(-rc)
            };
            return Err(StoreError::io(job.path, err));
        }
        done += rc as usize;
    }
    Ok(())
}

enum Reader<'s> {
    Session(&'s mut UringSession),
    Serial,
}

impl Reader<'_> {
    fn run(&mut self, jobs: &mut [ReadJob<'_>]) -> Result<(), StoreError> {
        match self {
            Reader::Serial => jobs.iter_mut().try_for_each(|j| libc_complete(j, 0)),
            Reader::Session(session) => run_on_session(session, jobs),
        }
    }
}

/// Keep the ring full with `jobs` and harvest until all complete.
fn run_on_session(session: &mut UringSession, jobs: &mut [ReadJob<'_>]) -> Result<(), StoreError> {
    session.begin_batch()?;
    let epoch = session.epoch();
    let mut next = 0usize;
    let mut inflight = 0usize;
    while next < jobs.len() || inflight > 0 {
        while next < jobs.len() && session.free_sq() > 0 {
            let i = next;
            next += 1;
            if jobs[i].buf.is_empty() {
                continue;
            }
            let ud = uring_session::pack_ud(uring_session::KIND_INDEX_BUILD, epoch, i as u32);
            let job = &mut jobs[i];
            // SAFETY: `job.buf` lives until this batch drains (the caller's
            // drain guard outlives `jobs`).
            unsafe { session.push_pread(job.fd, job.off, &mut job.buf, ud) }?;
            inflight += 1;
        }
        if inflight == 0 {
            break;
        }
        session.sync_submission();
        let _ = session.submit();
        let cqes = session.harvest_ready()?;
        if cqes.is_empty() {
            session.submit_and_wait_one()?;
            continue;
        }
        for (ud, res) in cqes {
            let (kind, ep, slot) = uring_session::unpack_ud(ud);
            let i = slot as usize;
            if kind != uring_session::KIND_INDEX_BUILD || ep != epoch || i >= jobs.len() {
                return Err(StoreError::Corrupt(
                    "invariant: io_uring index build unexpected cqe",
                ));
            }
            inflight -= 1;
            let job = &mut jobs[i];
            if res < 0 || res as usize != job.buf.len() {
                libc_complete(job, res.max(0) as usize)?;
            }
        }
    }
    Ok(())
}

/// Move a plan's buffers into `jobs` (in the plan's read order).
fn take_reads<'p, 'b>(
    jobs: &mut Vec<ReadJob<'p>>,
    file: (IoHandle, &'p Path),
    reads: impl Iterator<Item = (u64, &'b mut Vec<u8>)>,
) -> std::ops::Range<usize> {
    let start = jobs.len();
    for (off, buf) in reads {
        jobs.push(ReadJob {
            fd: file.0,
            path: file.1,
            off,
            buf: std::mem::take(buf),
        });
    }
    start..jobs.len()
}

/// Move filled buffers back into the plan (same read order as `take_reads`).
fn give_back<'b>(
    jobs: &mut [ReadJob<'_>],
    range: std::ops::Range<usize>,
    reads: impl Iterator<Item = (u64, &'b mut Vec<u8>)>,
) {
    for ((_, buf), job) in reads.zip(&mut jobs[range]) {
        *buf = std::mem::take(&mut job.buf);
    }
}

/// Read `heights` (consecutive) and every parent their creates spend.
pub fn read_index_window(
    table: &TxTable,
    heights: &[IndexHeight],
) -> Result<IndexWindow, StoreError> {
    match uring_session::with_thread_local(ENTRIES, |s| {
        let mut s = s.drain_guard();
        read_window(table, heights, &mut Reader::Session(&mut s))
    }) {
        Ok(r) => r,
        Err(StoreError::Unavailable) => read_window(table, heights, &mut Reader::Serial),
        Err(e) => Err(e),
    }
}

fn missing(what: &'static str) -> StoreError {
    StoreError::Corrupt(what)
}

fn read_window(
    table: &TxTable,
    heights: &[IndexHeight],
    reader: &mut Reader<'_>,
) -> Result<IndexWindow, StoreError> {
    let want_txids = heights.iter().any(|h| h.tweaks);
    let secret = Some(&table.secret);
    let body_file = (table.body.body_read_fd(), table.body.body_file_path());
    let sws_file = (
        table.seqsigwit.body_read_fd(),
        table.seqsigwit.body_file_path(),
    );
    let txid_file = (table.txids.body_read_fd(), table.txids.file_path());
    let (input_loc_file, input_body_file) = table.input.files();

    // Stage 1: locators.
    let fks: Vec<Fk> = heights
        .iter()
        .flat_map(|h| (0..u64::from(h.n)).map(move |i| Fk(h.first.0 + i)))
        .collect();
    let tw_fks: Vec<Fk> = heights
        .iter()
        .filter(|h| h.tweaks)
        .flat_map(|h| (0..u64::from(h.n)).map(move |i| Fk(h.first.0 + i)))
        .collect();
    let mut loc_plan = table.create_loc.plan_range_batch(&fks)?;
    let mut sws_plan = table.seqsigwit_loc.plan_range_batch(&tw_fks)?;
    let mut edge_plans = heights
        .iter()
        .map(|h| {
            table
                .input
                .plan_edges_span(h.first.0, h.first.0 + u64::from(h.n) - 1)
        })
        .collect::<Result<Vec<_>, _>>()?;
    {
        let mut jobs = Vec::new();
        let loc_r = take_reads(&mut jobs, table.create_loc.loc_file(), loc_plan.reads());
        let sws_r = take_reads(&mut jobs, table.seqsigwit_loc.loc_file(), sws_plan.reads());
        let edge_r = take_reads(
            &mut jobs,
            input_loc_file,
            edge_plans.iter_mut().map(|p| (p.loc_off, &mut p.loc)),
        );
        reader.run(&mut jobs)?;
        give_back(&mut jobs, loc_r, loc_plan.reads());
        give_back(&mut jobs, sws_r, sws_plan.reads());
        give_back(
            &mut jobs,
            edge_r,
            edge_plans.iter_mut().map(|p| (p.loc_off, &mut p.loc)),
        );
    }
    let locs = table.create_loc.finish_range_batch(&loc_plan)?;
    let sws_ranges = table.seqsigwit_loc.finish_range_batch(&sws_plan)?;

    // Stage 2: block spans.
    let mut jobs = Vec::new();
    let mut at = 0usize;
    let mut spans = Vec::with_capacity(heights.len());
    for (h, plan) in heights.iter().zip(&edge_plans) {
        let pairs = &locs[at..at + h.n as usize];
        at += h.n as usize;
        let pairs = pairs
            .iter()
            .map(|p| p.ok_or_else(|| missing("invariant: index build create loc missing")))
            .collect::<Result<Vec<_>, _>>()?;
        let lo = pairs.iter().map(|p| p.txout.0).min().unwrap_or(0);
        let hi = pairs
            .iter()
            .map(|p| p.txout.0 + p.txout.1)
            .max()
            .unwrap_or(0);
        let txout_j = jobs.len();
        jobs.push(ReadJob::new(body_file, lo, hi - lo));
        let (abs, len) = table.input.edges_body_read(plan)?;
        jobs.push(ReadJob::new(input_body_file, abs, len));
        if want_txids {
            let off = TxidBody::entry_offset(h.first.0)?;
            jobs.push(ReadJob::new(
                txid_file,
                off,
                u64::from(h.n) * TXID_ENTRY_LEN,
            ));
        }
        spans.push((txout_j, lo, pairs));
    }
    reader.run(&mut jobs)?;

    let mut blocks = Vec::with_capacity(heights.len());
    let mut in_window: U64Map<(usize, usize)> = U64Map::default();
    for (bi, ((h, plan), (txout_j, lo, pairs))) in
        heights.iter().zip(&edge_plans).zip(spans).enumerate()
    {
        let edges = decode_edges_span(plan, &jobs[txout_j + 1].buf)?
            .into_iter()
            .map(|e| e.ok_or_else(|| missing("invariant: index build input edges unstamped")))
            .collect::<Result<Vec<_>, _>>()?;
        let txids = want_txids.then(|| &jobs[txout_j + 2].buf);
        let mut txs = Vec::with_capacity(h.n as usize);
        for (ti, pair) in pairs.iter().enumerate() {
            let fk = Fk(h.first.0 + ti as u64);
            let rel = (pair.txout.0 - lo) as usize;
            let raw = &jobs[txout_j].buf[rel..rel + pair.txout.1 as usize];
            let (mut rec, outs, _) =
                decode_packed_tx_outs_with_spender_rels_secret(raw, pair.n_out, secret)?;
            if let Some(ids) = txids {
                rec.txid.copy_from_slice(&ids[ti * 32..ti * 32 + 32]);
            }
            rec.input_count = edges[ti].len() as u32;
            let need_seqsigwit = h.tweaks && outs.iter().any(|o| is_p2tr(&o.script));
            in_window.insert(fk.0, (bi, ti));
            txs.push(LoadedTweakTx {
                fk,
                rec,
                outs,
                need_seqsigwit,
                inputs: None,
            });
        }
        blocks.push(IndexBlock {
            height: h.height,
            header_fk: h.header_fk,
            txs,
            edges,
        });
    }
    drop(jobs);

    // Stage 3: parent locators, tweak witnesses, parent txids.
    let mut parent_fks: Vec<u64> = blocks
        .iter()
        .flat_map(|b| b.edges.iter().flatten())
        .map(|e| e.parent.0)
        .filter(|&fk| fk != 0 && !in_window.contains_key(&fk))
        .collect();
    parent_fks.sort_unstable();
    parent_fks.dedup();
    let parent_fk_list: Vec<Fk> = parent_fks.iter().map(|&f| Fk(f)).collect();
    let mut ploc_plan = table.create_loc.plan_range_batch(&parent_fk_list)?;
    let mut jobs = Vec::new();
    let ploc_r = take_reads(&mut jobs, table.create_loc.loc_file(), ploc_plan.reads());
    let mut sws_jobs = Vec::new();
    let mut tw_i = 0usize;
    for (bi, (b, h)) in blocks.iter().zip(heights).enumerate() {
        if !h.tweaks {
            continue;
        }
        for (ti, tx) in b.txs.iter().enumerate() {
            let range = sws_ranges[tw_i];
            tw_i += 1;
            if tx.need_seqsigwit {
                let (off, len) =
                    range.ok_or_else(|| missing("invariant: index build seqsigwit loc missing"))?;
                sws_jobs.push((bi, ti, jobs.len()));
                jobs.push(ReadJob::new(sws_file, off, len));
            }
        }
    }
    let txid_j = jobs.len();
    if want_txids {
        for &fk in &parent_fks {
            jobs.push(ReadJob::new(
                txid_file,
                TxidBody::entry_offset(fk)?,
                TXID_ENTRY_LEN,
            ));
        }
    }
    reader.run(&mut jobs)?;
    give_back(&mut jobs, ploc_r, ploc_plan.reads());
    for (bi, ti, j) in sws_jobs {
        let tx = &mut blocks[bi].txs[ti];
        let mut ins = decode_seqsigwit_secret(&jobs[j].buf, tx.rec.input_count, secret)?;
        apply_input_edges(&mut ins, &blocks[bi].edges[ti])?;
        blocks[bi].txs[ti].inputs = Some(ins);
    }
    let parent_txids: Vec<[u8; 32]> = (0..parent_fks.len())
        .map(|k| {
            let mut id = [0u8; 32];
            if want_txids {
                id.copy_from_slice(&jobs[txid_j + k].buf);
            }
            id
        })
        .collect();
    drop(jobs);
    let plocs = table.create_loc.finish_range_batch(&ploc_plan)?;

    // Stage 4: parent records.
    let mut jobs = Vec::with_capacity(plocs.len());
    for p in &plocs {
        let p = p.ok_or_else(|| missing("invariant: index build parent loc missing"))?;
        jobs.push(ReadJob::new(body_file, p.txout.0, p.txout.1));
    }
    reader.run(&mut jobs)?;
    let mut parents = U64Map::default();
    for (k, (job, p)) in jobs.iter().zip(&plocs).enumerate() {
        let n_out = p.map(|p| p.n_out).unwrap_or(0);
        let (_, outs, _) = decode_packed_tx_outs_with_spender_rels_secret(&job.buf, n_out, secret)?;
        parents.insert(parent_fks[k], (parent_txids[k], outs));
    }
    Ok(IndexWindow {
        blocks,
        parents,
        in_window,
    })
}
