//! Checkpoints for the header chain past the download queue.
//!
//! The queue stays at [`ORDERED_HEADERS_SOFT_CAP`] and keeps filling along the
//! candidate. Headers beyond the queue are not written. Each look-ahead reply
//! records one checkpoint: hash, height, and total work.

use super::state::IbdWorkState;
use super::ORDERED_HEADERS_SOFT_CAP;
use crate::chain::ChainHub;
use bitcoin::block::Header;
use bitcoin::hashes::Hash;
use bitcoin::BlockHash;
use bitcoin::CompactTarget;
use bitcoin::Work;
use std::collections::HashSet;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Checkpoint {
    hash: BlockHash,
    height: u32,
    work: [u8; 32],
    /// Last header of this reply. Lets the next batch check `nBits` and time
    /// without a row in `header.body`.
    header: Option<Header>,
    /// Up to 11 timestamps ending at `header`, oldest first.
    times: Vec<u32>,
}

#[derive(Debug, Default)]
pub(crate) struct HeaderWalk {
    checkpoints: Vec<Checkpoint>,
    /// Confirmed tip, or the stored path tip, before any look-ahead checkpoint.
    base_hash: Option<BlockHash>,
    base_height: u32,
    base_work: [u8; 32],
    tip_hash: Option<BlockHash>,
    tip_height: u32,
    tip_work: [u8; 32],
    origin: bool,
    /// Look-ahead tips that every peer left empty before the work floor.
    dead_ends: HashSet<BlockHash>,
    /// Peers that have answered empty at the current tip.
    emptied: HashSet<usize>,
    /// Hash at the milestone height on this walk. Not a stored header row.
    milestone_hash: Option<BlockHash>,
    /// Checkpoint work has reached the milestone floor.
    proven: bool,
    /// Stored header hashes from this peer that are not on the download path.
    off_path: std::collections::HashMap<usize, HashSet<BlockHash>>,
    /// The caught-up line has already been logged.
    announced_done: bool,
    /// Peer and time of the header request a look-ahead reply must answer.
    asked_peer: Option<usize>,
    asked_at: Option<Instant>,
    /// Header at the walk tip, for the next batch's `nBits` and median time.
    tip_header: Option<Header>,
    /// Up to 11 timestamps ending at `tip_header`, oldest first.
    tip_times: Vec<u32>,
    /// Header that opened the current difficulty period.
    period_header: Option<Header>,
    period_height: u32,
    /// Last header in this period whose `nBits` are not the min-difficulty limit.
    full_diff_bits: Option<CompactTarget>,
    full_diff_height: u32,
    /// Confirmed chain already proved the anchor; stop stat'ing the sidecar.
    adopt_retired: bool,
}

/// How long a look-ahead answer may take before the next peer is asked.
/// Shorter than a stall cooldown: one headers reply, not a silent peer.
const LOOKAHEAD_ASK: Duration = Duration::from_secs(5);
const DEAD_END_CAP: usize = 1024;

impl HeaderWalk {
    pub(crate) fn tip_height(&self) -> u32 {
        self.tip_height
    }

    pub(crate) fn checkpoints(&self) -> &[Checkpoint] {
        &self.checkpoints
    }

    pub(crate) fn has_checkpoints(&self) -> bool {
        !self.checkpoints.is_empty()
    }

    /// Candidate tip, then earlier checkpoints thinned back toward the base.
    pub(crate) fn locator_hashes(&self) -> Vec<BlockHash> {
        let mut out = Vec::new();
        if let Some(h) = self.tip_hash {
            out.push(h);
        }
        let mut step = 1usize;
        let mut i = self.checkpoints.len();
        while i > 0 && out.len() < 8 {
            i = i.saturating_sub(step);
            let h = self.checkpoints[i].hash;
            if !out.contains(&h) {
                out.push(h);
            }
            if i == 0 {
                break;
            }
            step = step.saturating_mul(2);
        }
        out
    }
}

fn ensure_origin(st: &mut IbdWorkState, hub: &ChainHub) {
    if st.header_walk.origin {
        return;
    }
    let mut height = hub.tip_height().unwrap_or(0);
    let mut hash = hub.tip_hash();
    let mut work = hub
        .chain_work()
        .unwrap_or_else(|_| Work::from_be_bytes([0u8; 32]));
    let queue_tail = st
        .ordered
        .back()
        .and_then(|h| st.hash_height.get(h).copied());
    while let Some(next_h) = height.checked_add(1) {
        if queue_tail.is_some_and(|tail| next_h > tail) {
            break;
        }
        let Some(next) = st.height_to_hash.get(&next_h).copied() else {
            break;
        };
        let Some(hdr) = hub.header_of(&next) else {
            break;
        };
        if hash.is_some_and(|h| hdr.prev_blockhash != h) {
            break;
        }
        work = work + hdr.work();
        remember_milestone(st, hub, next_h, next, hdr.prev_blockhash, &hdr);
        height = next_h;
        hash = Some(next);
    }
    let work_be = work.to_be_bytes();
    st.header_walk.tip_height = height;
    st.header_walk.tip_hash = hash;
    st.header_walk.tip_work = work_be;
    st.header_walk.base_hash = hash;
    st.header_walk.base_height = height;
    st.header_walk.base_work = work_be;
    st.header_walk.origin = true;
    if let Some(h) = hash.and_then(|h| hub.header_of(&h)) {
        seed_tip_times(st, hub, h);
        note_period(st, hub, h, height);
        note_full_diff(st, hub, &h, height);
    }
}

fn below_floor(hub: &ChainHub, work: &[u8; 32]) -> bool {
    match hub.milestone.anchor {
        Some(anchor) => work < &anchor.min_work_be,
        None => false,
    }
}

/// Operator line for the header walk, about every 5 seconds.
pub(crate) fn log_status(st: &mut IbdWorkState, horizon: u32) {
    if !st.header_walk.origin || st.header_walk.announced_done {
        return;
    }
    let height = st.header_walk.tip_height;
    let pct = super::progress::ibd_pct(height, horizon.max(height));
    let work = if st.header_walk.proven { "ok" } else { "below" };
    let phase = if st.ordered.len() >= ORDERED_HEADERS_SOFT_CAP {
        "walk"
    } else {
        "write"
    };
    rbitcoin_log::info!(
        "ibd: headers height={height} ({pct}%) horizon={horizon} work={work} phase={phase}"
    );
    if st.header_walk.proven && st.max_peer_height > 0 && height >= st.max_peer_height {
        st.header_walk.announced_done = true;
        rbitcoin_log::info!("ibd: headers done height={height} horizon={horizon}");
    }
}

/// The queue is full and peers still advertise headers past the candidate.
pub(crate) fn wants_lookahead(st: &IbdWorkState) -> bool {
    st.ordered.len() >= ORDERED_HEADERS_SOFT_CAP && st.max_peer_height > st.header_walk.tip_height()
}

/// Height a header peer must advertise past: the walk tip while the queue is
/// full, otherwise the queue tail.
pub(crate) fn ask_above(st: &IbdWorkState) -> u32 {
    if st.ordered.len() >= ORDERED_HEADERS_SOFT_CAP {
        return st.header_walk.tip_height();
    }
    st.ordered
        .back()
        .and_then(|h| st.hash_height.get(h).copied())
        .unwrap_or(0)
}

/// A proven-chain reply that must not be written.
///
/// After the work floor, a heavier chain that passes the low-work check
/// replaces the checkpoints. The queue drops hashes that are not on it. A
/// low-work or lighter chain is not stored. Returns true when the reply was
/// consumed. A heavier chain whose queue has room is not consumed: the caller
/// stores it because those blocks will be fetched.
pub(crate) fn suppress_competing_chain(
    st: &mut IbdWorkState,
    hub: &ChainHub,
    peer: usize,
    headers: &[Header],
) -> bool {
    if headers.is_empty() || !pow_linked(headers) {
        return false;
    }
    let below = below_floor(hub, &st.header_walk.tip_work);
    if !st.header_walk.proven && !below {
        return false;
    }
    if !st.header_walk.origin {
        return false;
    }
    let prev = headers[0].prev_blockhash;
    if st.header_walk.tip_hash == Some(prev) || batch_ends_on_checkpoint(st, headers) {
        return false;
    }
    if st.ordered.len() < ORDERED_HEADERS_SOFT_CAP && st.ordered.back().copied() == Some(prev) {
        return false;
    }
    if headers.iter().any(|h| hub.header_below_anti_dos(h)) {
        return true;
    }
    let Some(parent) = work_at(st, hub, prev) else {
        return false;
    };
    let mut work = parent;
    let mut link = prev;
    for hdr in headers {
        if hdr.prev_blockhash != link {
            break;
        }
        work = work + hdr.work();
        link = hdr.block_hash();
    }
    if link == prev || work <= Work::from_be_bytes(st.header_walk.tip_work) {
        note_dead(st, link);
        return true;
    }
    if parent_header(st, hub, prev).is_none() {
        return true;
    }
    if !batch_context_ok(st, hub, headers) {
        punish_header_peer(st, peer);
        return true;
    }
    adopt_heavier(st, hub, prev, headers, work);
    st.ordered.len() >= ORDERED_HEADERS_SOFT_CAP
}

fn clear_off_path(st: &mut IbdWorkState) {
    st.header_walk.off_path.clear();
}

fn note_dead(st: &mut IbdWorkState, hash: BlockHash) {
    if st.header_walk.dead_ends.len() >= DEAD_END_CAP {
        st.header_walk.dead_ends.clear();
    }
    st.header_walk.dead_ends.insert(hash);
}

fn rewind_above(st: &mut IbdWorkState, hub: &ChainHub, height: u32) {
    while st
        .header_walk
        .checkpoints
        .last()
        .is_some_and(|c| c.height > height)
    {
        st.header_walk.checkpoints.pop();
    }
    if let Some(prev) = st.header_walk.checkpoints.last() {
        st.header_walk.tip_hash = Some(prev.hash);
        st.header_walk.tip_height = prev.height;
        st.header_walk.tip_work = prev.work;
    } else {
        st.header_walk.tip_hash = st.header_walk.base_hash;
        st.header_walk.tip_height = st.header_walk.base_height;
        st.header_walk.tip_work = st.header_walk.base_work;
    }
    let keep = st.header_walk.tip_height;
    forget_milestone_below(st, hub, keep);
    restore_tip_header(st, hub);
    let drop: Vec<BlockHash> = st
        .ordered
        .iter()
        .copied()
        .filter(|h| st.hash_height.get(h).is_some_and(|ht| *ht > keep))
        .collect();
    forget_queue_hashes(st, &drop);
    clear_off_path(st);
    save_adopt(st, hub);
}

fn batch_ends_on_checkpoint(st: &IbdWorkState, headers: &[Header]) -> bool {
    let mut hash = headers[0].prev_blockhash;
    for hdr in headers {
        if hdr.prev_blockhash != hash {
            break;
        }
        hash = hdr.block_hash();
    }
    st.header_walk.tip_hash == Some(hash)
        || st.header_walk.checkpoints.iter().any(|c| c.hash == hash)
}

fn work_at(st: &IbdWorkState, hub: &ChainHub, hash: BlockHash) -> Option<Work> {
    if st.header_walk.base_hash == Some(hash) {
        return Some(Work::from_be_bytes(st.header_walk.base_work));
    }
    if let Some(c) = st.header_walk.checkpoints.iter().find(|c| c.hash == hash) {
        return Some(Work::from_be_bytes(c.work));
    }
    if hub.tip_hash() == Some(hash) {
        return hub.chain_work().ok();
    }
    None
}

fn adopt_heavier(
    st: &mut IbdWorkState,
    hub: &ChainHub,
    prev: BlockHash,
    headers: &[Header],
    work: Work,
) {
    let fork_height = if st.header_walk.base_hash == Some(prev) {
        st.header_walk.base_height
    } else {
        st.header_walk
            .checkpoints
            .iter()
            .find(|c| c.hash == prev)
            .map(|c| c.height)
            .unwrap_or(st.header_walk.base_height)
    };
    forget_milestone_below(st, hub, fork_height);
    st.header_walk
        .checkpoints
        .retain(|c| c.height <= fork_height);
    seed_fork_times(st, hub, prev);
    let mut height = fork_height;
    let mut link = prev;
    let mut hash = prev;
    let mut last = None;
    for hdr in headers {
        if hdr.prev_blockhash != link {
            break;
        }
        height = height.saturating_add(1);
        hash = hdr.block_hash();
        remember_milestone(st, hub, height, hash, link, hdr);
        link = hash;
        push_tip_time(st, *hdr);
        note_period(st, hub, *hdr, height);
        note_full_diff(st, hub, hdr, height);
        last = Some(*hdr);
    }
    let work_be = work.to_be_bytes();
    st.header_walk.tip_hash = Some(hash);
    st.header_walk.tip_height = height;
    st.header_walk.tip_work = work_be;
    st.header_walk.checkpoints.push(Checkpoint {
        hash,
        height,
        work: work_be,
        header: last,
        times: st.header_walk.tip_times.clone(),
    });
    publish_work(st, hub);
    st.header_walk.emptied.clear();
    st.header_walk.dead_ends.clear();
    clear_off_path(st);
    save_adopt(st, hub);
    let keep: HashSet<BlockHash> = headers.iter().map(|h| h.block_hash()).collect();
    let drop: Vec<BlockHash> = st
        .ordered
        .iter()
        .copied()
        .filter(|h| {
            if keep.contains(h) {
                return false;
            }
            !matches!(st.hash_height.get(h), Some(ht) if *ht <= fork_height)
        })
        .collect();
    forget_queue_hashes(st, &drop);
}

fn forget_queue_hashes(st: &mut IbdWorkState, drop: &[BlockHash]) {
    if drop.is_empty() {
        return;
    }
    let gone: HashSet<BlockHash> = drop.iter().copied().collect();
    st.ordered.retain(|h| !gone.contains(h));
    for h in &gone {
        st.ordered_set.remove(h);
        st.known_headers.remove(h);
        if let Some(ht) = st.hash_height.remove(h) {
            if st.height_to_hash.get(&ht) == Some(h) {
                st.height_to_hash.remove(&ht);
            }
        }
    }
}

/// Record a look-ahead batch as a checkpoint. True when the batch was consumed
/// and must not be written.
pub(crate) fn absorb_lookahead(
    st: &mut IbdWorkState,
    hub: &ChainHub,
    peer: usize,
    solicited: bool,
    headers: &[Header],
) -> bool {
    if headers.is_empty() || st.ordered.len() < ORDERED_HEADERS_SOFT_CAP {
        return false;
    }
    ensure_origin(st, hub);
    let Some(tip) = st.header_walk.tip_hash else {
        return false;
    };
    if st.header_walk.dead_ends.contains(&tip) {
        return true;
    }
    if headers[0].prev_blockhash != tip {
        return false;
    }
    if !solicited {
        return true;
    }
    if !batch_context_ok(st, hub, headers) {
        punish_header_peer(st, peer);
        st.header_walk.asked_peer = None;
        return true;
    }
    let extended = extend_tip(st, hub, headers);
    st.header_walk.asked_peer = None;
    extended
}

/// A batch that does not extend the live candidate below the work floor.
///
/// Building on a dead end, or on an earlier checkpoint while the candidate is
/// still live, must not be written.
pub(crate) fn ignore_below_floor(st: &IbdWorkState, hub: &ChainHub, headers: &[Header]) -> bool {
    if headers.is_empty() || !st.header_walk.origin || !below_floor(hub, &st.header_walk.tip_work) {
        return false;
    }
    let prev = headers[0].prev_blockhash;
    if st.header_walk.dead_ends.contains(&prev) {
        return true;
    }
    let Some(tip) = st.header_walk.tip_hash else {
        return false;
    };
    if prev == tip {
        return false;
    }
    if st.ordered.len() < ORDERED_HEADERS_SOFT_CAP && st.ordered.back().copied() == Some(prev) {
        return false;
    }
    true
}

/// A queue refill below or above the floor that does not land on the next
/// checkpoint. True when the reply was consumed and must not be written.
pub(crate) fn reject_refill_miss(
    st: &mut IbdWorkState,
    hub: &ChainHub,
    headers: &[Header],
) -> bool {
    if headers.is_empty()
        || !st.header_walk.origin
        || st.header_walk.checkpoints.is_empty()
        || st.ordered.len() >= ORDERED_HEADERS_SOFT_CAP
    {
        return false;
    }
    let Some(tail) = st.ordered.back().copied() else {
        return false;
    };
    if headers[0].prev_blockhash != tail {
        return false;
    }
    let Some(&tail_h) = st.hash_height.get(&tail) else {
        return false;
    };
    if !refill_misses_checkpoint(st, tail_h, headers) {
        return false;
    }
    rewind_above(st, hub, tail_h);
    false
}

fn refill_misses_checkpoint(st: &IbdWorkState, tail_h: u32, headers: &[Header]) -> bool {
    let checkpoints = st.header_walk.checkpoints();
    let Some(mut next_i) = checkpoints.iter().position(|c| c.height > tail_h) else {
        return false;
    };
    let mut matched = false;
    let mut height = tail_h;
    let mut prev = headers[0].prev_blockhash;
    for hdr in headers {
        if hdr.prev_blockhash != prev {
            break;
        }
        height = height.saturating_add(1);
        if height == checkpoints[next_i].height {
            if hdr.block_hash() != checkpoints[next_i].hash {
                return true;
            }
            matched = true;
            next_i += 1;
            if next_i >= checkpoints.len() {
                return false;
            }
        }
        prev = hdr.block_hash();
    }
    !matched
}

/// Every alive peer answered empty before the work floor. True when the walk
/// consumed the reply.
pub(crate) fn note_empty(st: &mut IbdWorkState, hub: &ChainHub, peer: usize) -> bool {
    if !st.header_walk.origin || !below_floor(hub, &st.header_walk.tip_work) {
        return false;
    }
    st.header_walk.emptied.insert(peer);
    let tall: Vec<usize> = st
        .slots
        .iter()
        .filter(|s| s.alive && s.peer_height > st.header_walk.tip_height)
        .map(|s| s.id)
        .collect();
    let alive: Vec<usize> = if tall.is_empty() {
        st.slots.iter().filter(|s| s.alive).map(|s| s.id).collect()
    } else {
        tall
    };
    if alive.is_empty() || alive.iter().any(|id| !st.header_walk.emptied.contains(id)) {
        ask_from_candidate(st, hub);
        return true;
    }
    rewind(st, hub);
    ask_from_candidate(st, hub);
    true
}

/// Headers stored because the queue had room. One checkpoint at the batch end
/// when they extend the candidate.
///
/// Context was already checked by [`ChainHub::ensure_headers_batch`]. A
/// look-ahead batch is not stored; it checks context in [`absorb_lookahead`]
/// and [`suppress_competing_chain`].
pub(crate) fn note_stored_candidate(st: &mut IbdWorkState, hub: &ChainHub, headers: &[Header]) {
    if !st.header_walk.origin || headers.is_empty() || !pow_linked(headers) {
        return;
    }
    let Some(tip) = st.header_walk.tip_hash else {
        return;
    };
    if headers[0].prev_blockhash != tip || st.header_walk.dead_ends.contains(&tip) {
        return;
    }
    let _ = extend_tip(st, hub, headers);
}

fn extend_tip(st: &mut IbdWorkState, hub: &ChainHub, headers: &[Header]) -> bool {
    let Some(tip) = st.header_walk.tip_hash else {
        return false;
    };
    let mut work = Work::from_be_bytes(st.header_walk.tip_work);
    let mut height = st.header_walk.tip_height;
    let mut prev = tip;
    let mut hash = tip;
    let mut last = None;
    for hdr in headers {
        if hdr.prev_blockhash != prev {
            break;
        }
        work = work + hdr.work();
        height = height.saturating_add(1);
        hash = hdr.block_hash();
        remember_milestone(st, hub, height, hash, prev, hdr);
        prev = hash;
        push_tip_time(st, *hdr);
        note_period(st, hub, *hdr, height);
        note_full_diff(st, hub, hdr, height);
        last = Some(*hdr);
    }
    if hash == tip {
        return false;
    }
    st.header_walk.tip_hash = Some(hash);
    st.header_walk.tip_height = height;
    st.header_walk.tip_work = work.to_be_bytes();
    st.header_walk.checkpoints.push(Checkpoint {
        hash,
        height,
        work: st.header_walk.tip_work,
        header: last,
        times: st.header_walk.tip_times.clone(),
    });
    st.header_walk.emptied.clear();
    publish_work(st, hub);
    save_adopt(st, hub);
    true
}

fn remember_milestone(
    st: &mut IbdWorkState,
    hub: &ChainHub,
    height: u32,
    hash: BlockHash,
    prev: BlockHash,
    hdr: &Header,
) {
    if hub.milestone.height == 0 || height != hub.milestone.height {
        return;
    }
    if let Some(existing) = st.header_walk.milestone_hash {
        if existing != hash {
            rbitcoin_log::warn!(
                "ibd: headers anchor mismatch height={height} hash={hash} previous={existing}"
            );
            return;
        }
    }
    st.header_walk.milestone_hash = Some(hash);
    hub.query.note_milestone_header(
        height,
        hash.to_byte_array(),
        prev.to_byte_array(),
        hdr.work(),
        None,
    );
}

fn publish_work(st: &mut IbdWorkState, hub: &ChainHub) {
    if below_floor(hub, &st.header_walk.tip_work) {
        return;
    }
    let opened = !st.header_walk.proven;
    hub.query
        .note_milestone_checkpoint_work(st.header_walk.tip_height, st.header_walk.tip_work);
    st.header_walk.proven = true;
    if !opened {
        return;
    }
    let Some(anchor) = hub.milestone.anchor else {
        return;
    };
    let Some(hash) = st.header_walk.milestone_hash else {
        return;
    };
    if hash != anchor.hash {
        rbitcoin_log::warn!(
            "ibd: headers anchor mismatch height={} hash={hash} anchor={}",
            hub.milestone.height,
            anchor.hash
        );
        return;
    }
    rbitcoin_log::info!(
        "ibd: headers milestone height={} hash={hash} work=ok",
        hub.milestone.height
    );
}

fn rewind(st: &mut IbdWorkState, hub: &ChainHub) {
    let Some(dead) = st.header_walk.tip_hash else {
        return;
    };
    let dead_height = st.header_walk.tip_height;
    note_dead(st, dead);
    st.header_walk.checkpoints.pop();
    if let Some(prev) = st.header_walk.checkpoints.last().cloned() {
        st.header_walk.tip_hash = Some(prev.hash);
        st.header_walk.tip_height = prev.height;
        st.header_walk.tip_work = prev.work;
    } else {
        st.header_walk.tip_hash = st.header_walk.base_hash;
        st.header_walk.tip_height = st.header_walk.base_height;
        st.header_walk.tip_work = st.header_walk.base_work;
    }
    let keep = st.header_walk.tip_height;
    forget_milestone_below(st, hub, keep);
    restore_tip_header(st, hub);
    drop_queue_past(st, keep, dead);
    st.header_walk.emptied.clear();
    clear_off_path(st);
    rbitcoin_log::warn!("ibd: headers dead-end hash={dead} height={dead_height} rewind={keep}");
    save_adopt(st, hub);
}

fn drop_queue_past(st: &mut IbdWorkState, keep_height: u32, dead: BlockHash) {
    let drop: Vec<BlockHash> = st
        .ordered
        .iter()
        .copied()
        .filter(|h| *h == dead || st.hash_height.get(h).is_some_and(|ht| *ht > keep_height))
        .collect();
    if drop.is_empty() {
        return;
    }
    forget_queue_hashes(st, &drop);
}

fn ask_from_candidate(st: &mut IbdWorkState, hub: &ChainHub) {
    let _ = send_getheaders(st, hub);
}

fn forget_milestone_below(st: &mut IbdWorkState, hub: &ChainHub, height: u32) {
    if hub.milestone.height == 0 || height >= hub.milestone.height {
        return;
    }
    st.header_walk.milestone_hash = None;
    hub.query.clear_milestone_path_above(height);
}

fn asked_fresh(st: &IbdWorkState, peer: usize) -> bool {
    if st.header_walk.asked_peer != Some(peer) {
        return false;
    }
    st.header_walk
        .asked_at
        .is_some_and(|t| t.elapsed() <= LOOKAHEAD_ASK)
}

/// The asked peer answered. Returns whether that ask was still inside the window.
pub(crate) fn take_header_ask(st: &mut IbdWorkState, peer: usize) -> bool {
    let fresh = asked_fresh(st, peer);
    if st.header_walk.asked_peer == Some(peer) {
        st.header_walk.asked_peer = None;
    }
    fresh
}

fn lookahead_pending(st: &IbdWorkState) -> bool {
    st.ordered.len() >= ORDERED_HEADERS_SOFT_CAP && asked_fresh_any(st)
}

fn asked_fresh_any(st: &IbdWorkState) -> bool {
    st.header_walk.asked_peer.is_some()
        && st
            .header_walk
            .asked_at
            .is_some_and(|t| t.elapsed() <= LOOKAHEAD_ASK)
}

fn punish_header_peer(st: &mut IbdWorkState, peer: usize) {
    super::dial::disconnect_peer(
        &mut st.slots,
        &mut st.addr_cooldown,
        &mut st.addr_strikes,
        peer,
    );
}

/// Ask for headers and remember which peer must answer a look-ahead.
pub(crate) fn send_getheaders(
    st: &mut IbdWorkState,
    hub: &ChainHub,
) -> Result<bool, crate::error::NetError> {
    if lookahead_pending(st) {
        return Ok(true);
    }
    let tips = super::path::work_path_tips(st);
    let above = ask_above(st);
    let Some(peer) =
        super::dial::request_headers(&st.slots, hub, &mut st.header_req_seq, &tips, above)?
    else {
        return Ok(false);
    };
    if st.ordered.len() >= ORDERED_HEADERS_SOFT_CAP {
        st.header_walk.asked_peer = Some(peer);
        st.header_walk.asked_at = Some(Instant::now());
    } else {
        st.header_walk.asked_peer = None;
    }
    Ok(true)
}

fn parent_header(st: &IbdWorkState, hub: &ChainHub, hash: BlockHash) -> Option<Header> {
    if st
        .header_walk
        .tip_header
        .is_some_and(|h| h.block_hash() == hash)
    {
        return st.header_walk.tip_header;
    }
    if let Some(hdr) = st
        .header_walk
        .checkpoints
        .iter()
        .rev()
        .find(|c| c.hash == hash)
        .and_then(|c| c.header)
    {
        return Some(hdr);
    }
    hub.header_of(&hash)
}

fn height_of(st: &IbdWorkState, hub: &ChainHub, hash: BlockHash) -> Option<u32> {
    if st.header_walk.tip_hash == Some(hash) {
        return Some(st.header_walk.tip_height);
    }
    if st.header_walk.base_hash == Some(hash) {
        return Some(st.header_walk.base_height);
    }
    if let Some(c) = st.header_walk.checkpoints.iter().find(|c| c.hash == hash) {
        return Some(c.height);
    }
    if let Some(h) = st.hash_height.get(&hash) {
        return Some(*h);
    }
    hub.query
        .height_of_hash(hash.as_byte_array())
        .ok()
        .flatten()
        .map(|h| h.0)
}

fn seed_tip_times(st: &mut IbdWorkState, hub: &ChainHub, header: Header) {
    let mut times = vec![header.time];
    let mut prev_hash = header.prev_blockhash;
    for _ in 0..10 {
        let Some(prev) = parent_header(st, hub, prev_hash) else {
            break;
        };
        times.push(prev.time);
        prev_hash = prev.prev_blockhash;
    }
    times.reverse();
    st.header_walk.tip_header = Some(header);
    st.header_walk.tip_times = times;
}

fn push_tip_time(st: &mut IbdWorkState, header: Header) {
    st.header_walk.tip_times.push(header.time);
    if st.header_walk.tip_times.len() > 11 {
        st.header_walk.tip_times.remove(0);
    }
    st.header_walk.tip_header = Some(header);
}

fn note_period(st: &mut IbdWorkState, hub: &ChainHub, header: Header, height: u32) {
    let interval = hub.params.difficulty_adjustment_interval();
    if interval > 0 && height.is_multiple_of(interval) {
        st.header_walk.period_header = Some(header);
        st.header_walk.period_height = height;
    }
}

fn seed_fork_times(st: &mut IbdWorkState, hub: &ChainHub, prev: BlockHash) {
    let saved = st
        .header_walk
        .checkpoints
        .iter()
        .find(|c| c.hash == prev)
        .and_then(|c| c.header.map(|h| (h, c.times.clone())));
    if let Some((header, times)) = saved {
        st.header_walk.tip_header = Some(header);
        st.header_walk.tip_times = times;
        return;
    }
    if let Some(parent) = parent_header(st, hub, prev) {
        seed_tip_times(st, hub, parent);
    }
}

fn restore_tip_header(st: &mut IbdWorkState, hub: &ChainHub) {
    let saved = st
        .header_walk
        .checkpoints
        .last()
        .map(|c| (c.header, c.times.clone(), c.hash));
    if let Some((header, times, hash)) = saved {
        if let Some(header) = header {
            st.header_walk.tip_header = Some(header);
            st.header_walk.tip_times = times;
            return;
        }
        if let Some(header) = hub.header_of(&hash) {
            seed_tip_times(st, hub, header);
            return;
        }
    } else if let Some(header) = st.header_walk.base_hash.and_then(|h| hub.header_of(&h)) {
        seed_tip_times(st, hub, header);
        return;
    }
    st.header_walk.tip_header = None;
    st.header_walk.tip_times.clear();
}

fn note_full_diff(st: &mut IbdWorkState, hub: &ChainHub, header: &Header, height: u32) {
    let limit = hub.params.pow_limit.to_compact_lossy();
    if header.bits != limit {
        st.header_walk.full_diff_bits = Some(header.bits);
        st.header_walk.full_diff_height = height;
    }
}

fn batch_context_ok(st: &IbdWorkState, hub: &ChainHub, headers: &[Header]) -> bool {
    if headers.is_empty() {
        return false;
    }
    let Some(mut parent) = parent_header(st, hub, headers[0].prev_blockhash) else {
        return false;
    };
    let Some(mut height) = height_of(st, hub, parent.block_hash()) else {
        return false;
    };
    let mut times = if st
        .header_walk
        .tip_header
        .is_some_and(|h| h.block_hash() == parent.block_hash())
        && !st.header_walk.tip_times.is_empty()
    {
        st.header_walk.tip_times.clone()
    } else {
        let mut times = vec![parent.time];
        let mut prev_hash = parent.prev_blockhash;
        for _ in 0..10 {
            let Some(prev) = parent_header(st, hub, prev_hash) else {
                break;
            };
            times.push(prev.time);
            prev_hash = prev.prev_blockhash;
        }
        times.reverse();
        times
    };
    let mut parent_hash = parent.block_hash();
    for hdr in headers {
        let hash = hdr.block_hash();
        if hdr.prev_blockhash != parent_hash {
            return false;
        }
        height = height.saturating_add(1);
        if hub
            .params
            .checkpoint_at(rbitcoin_primitives::Height(height))
            .is_some_and(|want| want != hash)
        {
            return false;
        }
        let mtp = if times.len() < 11 {
            0
        } else {
            rbitcoin_primitives::median_time_past_times(&times)
        };
        parent_hash = hash;
        let Some(bits) = expected_lookahead_bits(hub, height, &parent, hdr.time, st) else {
            return false;
        };
        if rbitcoin_consensus::validate_header_on_parent(
            &hub.params,
            rbitcoin_primitives::Height(height),
            hdr,
            mtp,
            bits,
        )
        .is_err()
        {
            return false;
        }
        if times.len() == 11 {
            times.remove(0);
        }
        times.push(hdr.time);
        parent = *hdr;
    }
    true
}

fn expected_lookahead_bits(
    hub: &ChainHub,
    height: u32,
    parent: &Header,
    header_time: u32,
    st: &IbdWorkState,
) -> Option<CompactTarget> {
    rbitcoin_consensus::next_work_bits(
        &hub.params,
        height,
        parent.bits,
        parent.time,
        header_time,
        period_first_time(hub, height, st),
        |h| lookahead_bits_at(st, hub, h),
    )
}

fn period_first_time(hub: &ChainHub, height: u32, st: &IbdWorkState) -> Option<u32> {
    let params = &hub.params;
    let interval = params.difficulty_adjustment_interval();
    if interval == 0 || !height.is_multiple_of(interval) || params.no_pow_retargeting() {
        return None;
    }
    let start_h = height - interval;
    let first = if st.header_walk.period_height == start_h {
        st.header_walk.period_header
    } else {
        None
    };
    first
        .or_else(|| header_on_store(st, hub, start_h))
        .map(|h| h.time)
}

fn lookahead_bits_at(st: &IbdWorkState, hub: &ChainHub, height: u32) -> Option<CompactTarget> {
    let limit = hub.params.pow_limit.to_compact_lossy();
    if let Some(bits) = st.header_walk.full_diff_bits {
        if height == st.header_walk.full_diff_height {
            return Some(bits);
        }
        if height > st.header_walk.full_diff_height && bits != limit {
            return Some(limit);
        }
    }
    if let Some(hdr) = header_on_store(st, hub, height) {
        return Some(hdr.bits);
    }
    if hub.params.allow_min_difficulty_blocks() && st.header_walk.full_diff_bits.is_none() {
        return Some(limit);
    }
    None
}

fn header_on_store(st: &IbdWorkState, hub: &ChainHub, height: u32) -> Option<Header> {
    if st.header_walk.tip_height == height {
        if let Some(hdr) = st.header_walk.tip_header {
            return Some(hdr);
        }
    }
    if st.header_walk.base_height == height {
        return st.header_walk.base_hash.and_then(|h| hub.header_of(&h));
    }
    st.height_to_hash
        .get(&height)
        .and_then(|h| hub.header_of(h))
}

fn pow_linked(headers: &[Header]) -> bool {
    for (i, hdr) in headers.iter().enumerate() {
        if hdr.validate_pow(hdr.target()).is_err() {
            return false;
        }
        if i > 0 && hdr.prev_blockhash != headers[i - 1].block_hash() {
            return false;
        }
    }
    true
}

const ADOPT_MAGIC: &[u8; 8] = b"rbtchdr1";

fn adopt_path(hub: &ChainHub) -> std::path::PathBuf {
    hub.query.store().path().join("header.adopt")
}

fn write_header(buf: &mut Vec<u8>, header: Option<Header>) {
    let mut raw = [0u8; 80];
    if let Some(header) = header {
        let enc = bitcoin::consensus::serialize(&header);
        if enc.len() == raw.len() {
            raw.copy_from_slice(&enc);
        }
    }
    buf.extend_from_slice(&raw);
}

fn read_header(raw: &[u8]) -> Option<Header> {
    if raw.iter().all(|b| *b == 0) {
        return None;
    }
    bitcoin::consensus::deserialize(raw).ok()
}

fn base_conflicts(st: &IbdWorkState, hub: &ChainHub, walk: &HeaderWalk) -> bool {
    let Some(base) = walk.base_hash else {
        return false;
    };
    if st
        .height_to_hash
        .get(&walk.base_height)
        .is_some_and(|h| *h != base)
    {
        return true;
    }
    match hub
        .query
        .header_at_height(rbitcoin_primitives::Height(walk.base_height))
    {
        Ok(Some((_, rec))) => rec.hash != *base.as_byte_array(),
        _ => false,
    }
}

/// A confirmed reorg under the walk's base drops `header.adopt`. The file must
/// not bring back a hash from the chain that was disconnected. A reorg under
/// the milestone height also drops the latched hash, so the next chain can
/// record its own. A reorg that stays above the milestone puts that hash back
/// after the path clear.
pub(crate) fn on_confirmed_rewind(st: &mut IbdWorkState, hub: &ChainHub, lca_h: u32) {
    if st.header_walk.base_hash.is_some() && lca_h < st.header_walk.base_height {
        st.header_walk = HeaderWalk::default();
        let _ = std::fs::remove_file(adopt_path(hub));
        return;
    }
    if hub.milestone.height == 0 || !st.header_walk.origin {
        return;
    }
    if lca_h < hub.milestone.height {
        if st.header_walk.milestone_hash.take().is_some() {
            save_adopt(st, hub);
        }
        return;
    }
    let Some(hash) = st.header_walk.milestone_hash else {
        return;
    };
    hub.query.note_milestone_header(
        hub.milestone.height,
        hash.to_byte_array(),
        [0u8; 32],
        Work::from_be_bytes([0u8; 32]),
        None,
    );
}

fn save_adopt(st: &IbdWorkState, hub: &ChainHub) {
    if !st.header_walk.origin {
        return;
    }
    let mut buf = Vec::new();
    buf.extend_from_slice(ADOPT_MAGIC);
    let n = st.header_walk.checkpoints.len() as u32;
    buf.extend_from_slice(&n.to_le_bytes());
    for c in st.header_walk.checkpoints() {
        buf.extend_from_slice(c.hash.as_byte_array());
        buf.extend_from_slice(&c.height.to_le_bytes());
        buf.extend_from_slice(&c.work);
        write_header(&mut buf, c.header);
        write_times(&mut buf, &c.times);
    }
    let milestone = st
        .header_walk
        .milestone_hash
        .map(|h| *h.as_byte_array())
        .unwrap_or([0u8; 32]);
    buf.extend_from_slice(&milestone);
    let base = st
        .header_walk
        .base_hash
        .map(|h| *h.as_byte_array())
        .unwrap_or([0u8; 32]);
    buf.extend_from_slice(&base);
    buf.extend_from_slice(&st.header_walk.base_height.to_le_bytes());
    buf.extend_from_slice(&st.header_walk.base_work);
    write_header(&mut buf, st.header_walk.tip_header);
    buf.extend_from_slice(&st.header_walk.period_height.to_le_bytes());
    write_header(&mut buf, st.header_walk.period_header);
    buf.extend_from_slice(&st.header_walk.full_diff_height.to_le_bytes());
    let full_bits = st
        .header_walk
        .full_diff_bits
        .map(|b| b.to_consensus())
        .unwrap_or(0);
    buf.extend_from_slice(&full_bits.to_le_bytes());
    let path = adopt_path(hub);
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, &buf).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Drop `header.adopt` once the confirmed chain itself proves the anchor.
/// Until then the file is how a restart turns the script skip back on.
pub(crate) fn retire_adopt_if_confirmed(st: &mut IbdWorkState, hub: &ChainHub) {
    if st.header_walk.adopt_retired {
        return;
    }
    let Some(anchor) = hub.milestone.anchor else {
        return;
    };
    if hub.milestone.height == 0 {
        return;
    }
    let tip_h = hub.tip_height().unwrap_or(0);
    if tip_h < hub.milestone.height {
        return;
    }
    let Ok(Some((_, rec))) = hub
        .query
        .header_at_height(rbitcoin_primitives::Height(hub.milestone.height))
    else {
        return;
    };
    if rec.hash != *anchor.hash.as_byte_array() {
        return;
    }
    let Ok(work) = hub.chain_work() else {
        return;
    };
    if work.to_be_bytes() < anchor.min_work_be {
        return;
    }
    st.header_walk.adopt_retired = true;
    hub.query.note_milestone_header(
        hub.milestone.height,
        rec.hash,
        [0u8; 32],
        Work::from_be_bytes([0u8; 32]),
        None,
    );
    hub.query
        .note_milestone_checkpoint_work(tip_h, work.to_be_bytes());
    let _ = std::fs::remove_file(adopt_path(hub));
}

/// Read `header.adopt`. A file that does not parse leaves the script skip off.
pub(crate) fn restore_adopt(st: &mut IbdWorkState, hub: &ChainHub) -> bool {
    retire_adopt_if_confirmed(st, hub);
    let started = std::time::Instant::now();
    let path = adopt_path(hub);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
        Err(_) => {
            rbitcoin_log::info!("ibd: headers resume failed took={:?}", started.elapsed());
            return false;
        }
    };
    let took = started.elapsed();
    let Some(parsed) = parse_adopt(&bytes) else {
        rbitcoin_log::info!("ibd: headers resume failed took={took:?}");
        return false;
    };
    if base_conflicts(st, hub, &parsed) {
        rbitcoin_log::info!(
            "ibd: headers resume refused height={} took={took:?}",
            parsed.base_height
        );
        return false;
    }
    st.header_walk = parsed;
    restore_tip_header(st, hub);
    renote_stored_path(st, hub);
    if let Some(hash) = st.header_walk.milestone_hash {
        if hub.milestone.height > 0 {
            hub.query.note_milestone_header(
                hub.milestone.height,
                hash.to_byte_array(),
                [0u8; 32],
                Work::from_be_bytes([0u8; 32]),
                None,
            );
        }
    }
    if !below_floor(hub, &st.header_walk.tip_work) {
        hub.query
            .note_milestone_checkpoint_work(st.header_walk.tip_height, st.header_walk.tip_work);
        st.header_walk.proven = true;
    }
    let work = if st.header_walk.proven { "ok" } else { "below" };
    rbitcoin_log::info!(
        "ibd: headers resume height={} work={work} took={took:?}",
        st.header_walk.tip_height
    );
    true
}

fn parse_adopt(bytes: &[u8]) -> Option<HeaderWalk> {
    if bytes.len() < 8 + 4 + 32 || &bytes[..8] != ADOPT_MAGIC {
        return None;
    }
    let n = u32::from_le_bytes(bytes[8..12].try_into().ok()?) as usize;
    const CKPT: usize = 32 + 4 + 32 + 80 + 1 + 11 * 4;
    let body = 12 + n * CKPT;
    const TAIL: usize = 32 + 32 + 4 + 32 + 80 + 4 + 80 + 4 + 4;
    if bytes.len() != body + TAIL {
        return None;
    }
    let mut checkpoints = Vec::with_capacity(n);
    let mut off = 12;
    for _ in 0..n {
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&bytes[off..off + 32]);
        off += 32;
        let height = u32::from_le_bytes(bytes[off..off + 4].try_into().ok()?);
        off += 4;
        let mut work = [0u8; 32];
        work.copy_from_slice(&bytes[off..off + 32]);
        off += 32;
        let header = read_header(&bytes[off..off + 80]);
        off += 80;
        let times = read_times(&bytes[off..off + 1 + 11 * 4])?;
        off += 1 + 11 * 4;
        checkpoints.push(Checkpoint {
            hash: BlockHash::from_byte_array(hash),
            height,
            work,
            header,
            times,
        });
    }
    let mut milestone = [0u8; 32];
    milestone.copy_from_slice(&bytes[off..off + 32]);
    off += 32;
    let milestone_hash = (milestone != [0u8; 32]).then_some(BlockHash::from_byte_array(milestone));
    let mut base = [0u8; 32];
    base.copy_from_slice(&bytes[off..off + 32]);
    off += 32;
    let base_height = u32::from_le_bytes(bytes[off..off + 4].try_into().ok()?);
    off += 4;
    let mut base_work = [0u8; 32];
    base_work.copy_from_slice(&bytes[off..off + 32]);
    off += 32;
    let tip_header = read_header(&bytes[off..off + 80]);
    off += 80;
    let period_height = u32::from_le_bytes(bytes[off..off + 4].try_into().ok()?);
    off += 4;
    let period_header = read_header(&bytes[off..off + 80]);
    off += 80;
    let full_diff_height = u32::from_le_bytes(bytes[off..off + 4].try_into().ok()?);
    off += 4;
    let full_bits = u32::from_le_bytes(bytes[off..off + 4].try_into().ok()?);
    let full_diff_bits = (full_bits != 0).then_some(CompactTarget::from_consensus(full_bits));
    let (tip_hash, tip_height, tip_work) = match checkpoints.last() {
        Some(c) => (Some(c.hash), c.height, c.work),
        None => (None, 0, [0u8; 32]),
    };
    Some(HeaderWalk {
        checkpoints,
        base_hash: (base != [0u8; 32]).then_some(BlockHash::from_byte_array(base)),
        base_height,
        base_work,
        tip_hash,
        tip_height,
        tip_work,
        origin: true,
        dead_ends: HashSet::new(),
        emptied: HashSet::new(),
        milestone_hash,
        proven: false,
        off_path: std::collections::HashMap::new(),
        announced_done: false,
        asked_peer: None,
        asked_at: None,
        tip_header,
        tip_times: Vec::new(),
        period_header,
        period_height,
        full_diff_bits,
        full_diff_height,
        adopt_retired: false,
    })
}

const TIMES_SLOTS: usize = 11;

fn write_times(buf: &mut Vec<u8>, times: &[u32]) {
    let n = times.len().min(TIMES_SLOTS);
    let start = times.len().saturating_sub(TIMES_SLOTS);
    buf.push(n as u8);
    for i in 0..TIMES_SLOTS {
        let t = times.get(start + i).copied().unwrap_or(0);
        buf.extend_from_slice(&t.to_le_bytes());
    }
}

fn read_times(bytes: &[u8]) -> Option<Vec<u32>> {
    if bytes.len() < 1 + TIMES_SLOTS * 4 {
        return None;
    }
    let n = bytes[0] as usize;
    if n > TIMES_SLOTS {
        return None;
    }
    let mut times = Vec::with_capacity(n);
    for i in 0..n {
        let off = 1 + i * 4;
        times.push(u32::from_le_bytes(bytes[off..off + 4].try_into().ok()?));
    }
    Some(times)
}

/// Below the milestone work floor, more than this many stored headers that are
/// not on the download path disconnects the peer.
const OFF_PATH_HEADER_BUDGET: u32 = 4_000;

/// Count one stored header that is not on the download path. True when the
/// peer is over the budget and still below the work floor.
pub(crate) fn note_off_path(
    st: &mut IbdWorkState,
    hub: &ChainHub,
    peer: usize,
    hash: BlockHash,
) -> bool {
    if hub.milestone.anchor.is_none() {
        return false;
    }
    if st.header_walk.proven && !below_floor(hub, &st.header_walk.tip_work) {
        return false;
    }
    let seen = st.header_walk.off_path.entry(peer).or_default();
    if seen.len() > OFF_PATH_HEADER_BUDGET as usize {
        return true;
    }
    if !seen.insert(hash) {
        return false;
    }
    seen.len() > OFF_PATH_HEADER_BUDGET as usize
}

fn renote_stored_path(st: &IbdWorkState, hub: &ChainHub) {
    let mut height = hub.tip_height().unwrap_or(0);
    let mut prev = hub.tip_hash();
    while let Some(next_h) = height.checked_add(1) {
        let Some(next) = st.height_to_hash.get(&next_h).copied() else {
            break;
        };
        let Some(hdr) = hub.header_of(&next) else {
            break;
        };
        if prev.is_some_and(|p| hdr.prev_blockhash != p) {
            break;
        }
        let base = if hub.tip_hash() == prev {
            hub.chain_work().ok()
        } else {
            None
        };
        hub.query.note_milestone_header(
            next_h,
            next.to_byte_array(),
            hdr.prev_blockhash.to_byte_array(),
            hdr.work(),
            base,
        );
        height = next_h;
        prev = Some(next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ibd::events::apply_peer_event;
    use crate::ibd::peer_io::{PeerCmd, PeerEvent, PeerSlot};
    use crate::ibd::state::IbdWorkState;
    use crate::seeds::AddrMan;
    use bitcoin::block::Header;
    use bitcoin::CompactTarget;
    use bitcoin::TxMerkleNode;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::atomic::{AtomicU32, AtomicU64};
    use std::sync::Arc;
    use tokio::sync::mpsc;

    fn mine(prev: BlockHash, n: u32) -> Header {
        let mut h = Header {
            version: bitcoin::block::Version::from_consensus(4),
            prev_blockhash: prev,
            merkle_root: TxMerkleNode::from_byte_array([n as u8; 32]),
            time: 1_600_000_000 + n,
            bits: CompactTarget::from_consensus(0x207fffff),
            nonce: n,
        };
        rbitcoin_consensus::grind_regtest_pow(&mut h);
        h
    }

    fn slot(id: usize) -> (PeerSlot, mpsc::UnboundedReceiver<PeerCmd>) {
        let (cmd_tx, rx) = mpsc::unbounded_channel();
        let task = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .spawn(async {});
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 9, 0, id as u8)), 8333);
        (
            PeerSlot {
                id,
                addr,
                net: crate::NetAddr::from_socket(addr),
                cmd_tx,
                in_flight: Default::default(),
                peer_height: 50_000,
                connected_ms: 1,
                first_data_ms: 0,
                bytes_rx_total: Arc::new(AtomicU64::new(0)),
                rate: Default::default(),
                alive: true,
                task,
            },
            rx,
        )
    }

    fn fill_queue(st: &mut IbdWorkState) {
        for i in 0..ORDERED_HEADERS_SOFT_CAP {
            let mut b = [0u8; 32];
            b[28..32].copy_from_slice(&(i as u32).to_le_bytes());
            let h = BlockHash::from_byte_array(b);
            st.ordered.push_back(h);
            st.ordered_set.insert(h);
        }
    }

    fn apply(st: &mut IbdWorkState, hub: &ChainHub, peer: usize, headers: Vec<Header>) {
        apply_peer_event(
            st,
            hub,
            PeerEvent::Headers { peer, headers },
            &AtomicU32::new(0),
            &mut AddrMan::new(),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 1),
            None,
        );
    }

    fn plant_on_queue(st: &mut IbdWorkState, hash: BlockHash, height: u32) {
        let old = st.ordered.pop_back().unwrap();
        st.ordered_set.remove(&old);
        st.ordered.push_back(hash);
        st.ordered_set.insert(hash);
        st.hash_height.insert(hash, height);
    }

    #[test]
    fn full_queue_checkpoints_lookahead_and_refill_stores_the_successor() {
        let (_dir, hub) = crate::chain::tiny_regtest_hub_labeled("header-walk-1");
        hub.ensure_genesis().unwrap();
        let gen = hub.tip_hash().unwrap();
        let good = mine(gen, 1);
        let (s0, mut rx) = slot(0);
        let mut st = IbdWorkState::new(vec![s0], Some(gen), Some(0));
        fill_queue(&mut st);
        let before = hub.query.store().header_count();
        log_status(&mut st, 50_000);
        assert!(
            !st.header_walk.announced_done,
            "the status line stays quiet until the walk has a candidate"
        );

        assert!(send_getheaders(&mut st, &hub).unwrap());
        let _ = rx.try_recv();
        apply(&mut st, &hub, 0, vec![good]);
        assert_eq!(
            hub.query.store().header_count(),
            before,
            "look-ahead past a full queue is not written"
        );
        assert_eq!(st.ordered.len(), ORDERED_HEADERS_SOFT_CAP);
        assert_eq!(st.header_walk.checkpoints().len(), 1);
        assert_eq!(st.header_walk.tip_hash, Some(good.block_hash()));
        assert!(wants_lookahead(&st));

        assert!(send_getheaders(&mut st, &hub).unwrap());
        match rx.try_recv() {
            Ok(PeerCmd::GetHeaders { locator }) => {
                assert_eq!(
                    locator[0],
                    good.block_hash(),
                    "locator starts at the candidate"
                );
            }
            Ok(_) => panic!("expected getheaders"),
            Err(_) => panic!("peer was not asked for headers"),
        }

        let dropped = st.ordered.pop_front().unwrap();
        st.ordered_set.remove(&dropped);
        apply(&mut st, &hub, 0, vec![good]);
        assert_eq!(
            hub.query.store().header_count(),
            before + 1,
            "the queue stores the successor once it has room"
        );
        assert_eq!(st.ordered.len(), ORDERED_HEADERS_SOFT_CAP);
        assert!(st.ordered_set.contains(&good.block_hash()));

        log_status(&mut st, 50_000);
        assert!(
            !st.header_walk.announced_done,
            "peers still advertise headers past the candidate"
        );
        let tip = st.header_walk.tip_height;
        st.max_peer_height = 0;
        log_status(&mut st, tip);
        assert!(!st.header_walk.announced_done);
        st.max_peer_height = tip;
        log_status(&mut st, tip);
        assert!(
            st.header_walk.announced_done,
            "the done line fires once the header tip has caught the peers"
        );
        log_status(&mut st, tip);
        assert!(st.header_walk.announced_done);
    }

    #[test]
    fn short_chain_rewinds_and_the_next_peer_is_followed() {
        let (_dir, mut hub) = crate::chain::tiny_regtest_hub_labeled("header-walk-2");
        hub.ensure_genesis().unwrap();
        hub.milestone = rbitcoin_consensus::Milestone {
            height: 840_000,
            anchor: Some(rbitcoin_consensus::MilestoneAnchor {
                hash: BlockHash::from_byte_array([0xab; 32]),
                min_work_be: [0xff; 32],
            }),
        };
        let gen = hub.tip_hash().unwrap();
        let a1 = mine(gen, 1);
        let a2 = mine(a1.block_hash(), 2);
        let b1 = mine(gen, 11);
        let b2 = mine(b1.block_hash(), 12);
        let b3 = mine(b2.block_hash(), 13);
        let (s0, _rx0) = slot(0);
        let (s1, _rx1) = slot(1);
        let mut st = IbdWorkState::new(vec![s0, s1], Some(gen), Some(0));
        fill_queue(&mut st);
        let before = hub.query.store().header_count();

        assert!(send_getheaders(&mut st, &hub).unwrap());
        apply(&mut st, &hub, 0, vec![a1, a2]);
        assert_eq!(hub.query.store().header_count(), before);
        log_status(&mut st, 50_000);
        assert!(!st.header_walk.proven, "work is still below the floor");
        assert_eq!(st.header_walk.tip_hash, Some(a2.block_hash()));
        plant_on_queue(&mut st, a1.block_hash(), 1);
        plant_on_queue(&mut st, a2.block_hash(), 2);

        apply(&mut st, &hub, 0, vec![]);
        assert_eq!(
            st.header_walk.tip_hash,
            Some(a2.block_hash()),
            "one peer's empty does not abandon the candidate"
        );
        apply(&mut st, &hub, 1, vec![]);
        assert_eq!(st.header_walk.tip_hash, Some(gen));
        log_status(&mut st, 50_000);
        assert!(!st.header_walk.announced_done);
        assert!(!st.ordered_set.contains(&a1.block_hash()));
        assert!(!st.ordered_set.contains(&a2.block_hash()));
        assert!(hub
            .query
            .get_header_by_hash(a1.block_hash().as_byte_array())
            .unwrap()
            .is_none());
        assert!(hub
            .query
            .get_header_by_hash(a2.block_hash().as_byte_array())
            .unwrap()
            .is_none());

        apply(&mut st, &hub, 1, vec![b1, b2, b3]);
        assert_eq!(st.header_walk.tip_hash, Some(b3.block_hash()));
        assert!(st.ordered_set.contains(&b3.block_hash()));
        assert!(hub
            .query
            .get_header_by_hash(a2.block_hash().as_byte_array())
            .unwrap()
            .is_none());
        assert_eq!(hub.query.store().header_count(), before + 3);
    }

    fn skips(hub: &ChainHub, height: u32, hash: &[u8; 32]) -> bool {
        hub.milestone.skips_scripts(
            height,
            hash,
            |h| hub.query.milestone_header_at(h),
            hub.query.milestone_best_work_be(),
        )
    }

    fn fill_to_cap(st: &mut IbdWorkState) {
        let mut i = st.ordered.len();
        while st.ordered.len() < ORDERED_HEADERS_SOFT_CAP {
            let mut b = [0xee; 32];
            b[28..32].copy_from_slice(&(i as u32).to_le_bytes());
            let h = BlockHash::from_byte_array(b);
            st.ordered.push_back(h);
            st.ordered_set.insert(h);
            i += 1;
        }
    }

    #[test]
    fn milestone_checkpoint_opens_script_skip_without_the_lookahead_headers() {
        let (_dir, mut hub) = crate::chain::tiny_regtest_hub_labeled("header-walk-3");
        hub.ensure_genesis().unwrap();
        let gen = hub.tip_hash().unwrap();
        let h1 = mine(gen, 1);
        let h2 = mine(h1.block_hash(), 2);
        let h3 = mine(h2.block_hash(), 3);
        let mut floor = [0u8; 32];
        floor[31] = 1;
        hub.milestone = rbitcoin_consensus::Milestone {
            height: 2,
            anchor: Some(rbitcoin_consensus::MilestoneAnchor {
                hash: h2.block_hash(),
                min_work_be: floor,
            }),
        };
        let (s0, _rx0) = slot(0);
        let mut st = IbdWorkState::new(vec![s0], Some(gen), Some(0));
        apply(&mut st, &hub, 0, vec![h1]);
        let after_queue = hub.query.store().header_count();
        fill_to_cap(&mut st);
        assert!(send_getheaders(&mut st, &hub).unwrap());
        apply(&mut st, &hub, 0, vec![h2, h3]);

        assert_eq!(
            hub.query.store().header_count(),
            after_queue,
            "headers between the queue and the tip are not written"
        );
        assert!(hub
            .query
            .get_header_by_hash(h2.block_hash().as_byte_array())
            .unwrap()
            .is_none());
        assert!(st.ordered_set.contains(&h1.block_hash()));
        assert!(
            skips(&hub, 1, h1.block_hash().as_byte_array()),
            "the queued block skips when the checkpoint hash is the anchor"
        );
        assert!(!skips(&hub, 1, &[0x44; 32]));
        assert!(
            !skips(&hub, 2, &[0x55; 32]),
            "a different hash at the milestone height does not skip"
        );
    }

    #[test]
    fn heavier_chain_replaces_checkpoints_and_a_cheap_fork_is_not_stored() {
        let (_dir, mut hub) = crate::chain::tiny_regtest_hub_labeled("header-walk-4");
        hub.ensure_genesis().unwrap();
        let gen = hub.tip_hash().unwrap();
        let a1 = mine(gen, 1);
        let a2 = mine(a1.block_hash(), 2);
        let c2 = mine(a1.block_hash(), 21);
        let c3 = mine(c2.block_hash(), 22);
        let c4 = mine(c3.block_hash(), 23);
        let c5 = mine(c4.block_hash(), 24);
        let light = mine(gen, 31);
        let less = mine(a1.block_hash(), 41);
        let mut floor = [0u8; 32];
        floor[31] = 1;
        hub.milestone = rbitcoin_consensus::Milestone {
            height: 2,
            anchor: Some(rbitcoin_consensus::MilestoneAnchor {
                hash: a2.block_hash(),
                min_work_be: floor,
            }),
        };
        let (s0, _rx0) = slot(0);
        let mut st = IbdWorkState::new(vec![s0], Some(gen), Some(0));
        apply(&mut st, &hub, 0, vec![a1]);
        fill_to_cap(&mut st);
        assert!(send_getheaders(&mut st, &hub).unwrap());
        apply(&mut st, &hub, 0, vec![a2]);
        plant_on_queue(&mut st, a2.block_hash(), 2);
        let before = hub.query.store().header_count();
        hub.set_minimum_chain_work(Some([0xff; 32]));

        apply(&mut st, &hub, 0, vec![c2, c3, c4, c5]);
        assert_eq!(st.header_walk.tip_hash, Some(c5.block_hash()));
        assert!(!st.ordered_set.contains(&a2.block_hash()));
        assert_eq!(hub.query.store().header_count(), before + 4);
        assert!(st.ordered_set.contains(&c5.block_hash()));

        let after_heavy = hub.query.store().header_count();
        apply(&mut st, &hub, 0, vec![light]);
        apply(&mut st, &hub, 0, vec![less]);
        assert_eq!(hub.query.store().header_count(), after_heavy);
        assert_eq!(st.header_walk.tip_hash, Some(c5.block_hash()));
        assert!(hub
            .query
            .get_header_by_hash(light.block_hash().as_byte_array())
            .unwrap()
            .is_none());
        assert!(hub
            .query
            .get_header_by_hash(less.block_hash().as_byte_array())
            .unwrap()
            .is_none());
        assert!(hub
            .query
            .get_header_by_hash(a2.block_hash().as_byte_array())
            .unwrap()
            .is_none());
    }

    #[test]
    fn queue_stores_a_reply_that_ends_on_the_checkpoint() {
        let (_dir, hub) = crate::chain::tiny_regtest_hub_labeled("header-walk-5");
        hub.ensure_genesis().unwrap();
        let gen = hub.tip_hash().unwrap();
        let good = mine(gen, 1);
        let miss = mine(gen, 9);
        let (s0, mut rx) = slot(0);
        let mut st = IbdWorkState::new(vec![s0], Some(gen), Some(0));
        fill_queue(&mut st);
        let before = hub.query.store().header_count();
        assert!(send_getheaders(&mut st, &hub).unwrap());
        let _ = rx.try_recv();
        apply(&mut st, &hub, 0, vec![good]);
        assert_eq!(hub.query.store().header_count(), before);
        assert_eq!(st.header_walk.tip_hash, Some(good.block_hash()));

        while st.ordered.len() >= ORDERED_HEADERS_SOFT_CAP {
            let dropped = st.ordered.pop_front().unwrap();
            st.ordered_set.remove(&dropped);
        }
        let queue_tip = *st.ordered.back().unwrap();
        assert!(send_getheaders(&mut st, &hub).unwrap());
        match rx.try_recv() {
            Ok(PeerCmd::GetHeaders { locator }) => {
                assert_eq!(locator[0], queue_tip, "locator starts at the queue tip");
            }
            Ok(_) => panic!("expected getheaders"),
            Err(_) => panic!("peer was not asked for headers"),
        }

        apply(&mut st, &hub, 0, vec![good]);
        assert_eq!(hub.query.store().header_count(), before + 1);
        assert!(st.ordered_set.contains(&good.block_hash()));

        let after = hub.query.store().header_count();
        apply(&mut st, &hub, 0, vec![miss]);
        assert_eq!(hub.query.store().header_count(), after);
        assert!(st.header_walk.dead_ends.contains(&miss.block_hash()));
        assert_eq!(st.header_walk.tip_hash, Some(good.block_hash()));
        assert!(hub
            .query
            .get_header_by_hash(miss.block_hash().as_byte_array())
            .unwrap()
            .is_none());
    }

    #[test]
    fn restart_restores_the_checkpoint_skip_and_garbage_does_not() {
        let (_dir, mut hub) = crate::chain::tiny_regtest_hub_labeled("header-walk-6");
        hub.ensure_genesis().unwrap();
        let gen = hub.tip_hash().unwrap();
        let h1 = mine(gen, 1);
        let h2 = mine(h1.block_hash(), 2);
        let h3 = mine(h2.block_hash(), 3);
        let mut floor = [0u8; 32];
        floor[31] = 1;
        hub.milestone = rbitcoin_consensus::Milestone {
            height: 2,
            anchor: Some(rbitcoin_consensus::MilestoneAnchor {
                hash: h2.block_hash(),
                min_work_be: floor,
            }),
        };
        let (s0, _rx0) = slot(0);
        let mut st = IbdWorkState::new(vec![s0], Some(gen), Some(0));
        apply(&mut st, &hub, 0, vec![h1]);
        fill_to_cap(&mut st);
        assert!(send_getheaders(&mut st, &hub).unwrap());
        apply(&mut st, &hub, 0, vec![h2, h3]);
        assert!(skips(&hub, 1, h1.block_hash().as_byte_array()));
        assert!(hub
            .query
            .get_header_by_hash(h2.block_hash().as_byte_array())
            .unwrap()
            .is_none());

        st.header_walk = HeaderWalk::default();
        hub.query.clear_milestone_path_above(0);
        assert!(!skips(&hub, 1, h1.block_hash().as_byte_array()));
        assert!(restore_adopt(&mut st, &hub));
        assert!(
            skips(&hub, 1, h1.block_hash().as_byte_array()),
            "the skip is on again from header.adopt without the look-ahead rows"
        );
        assert_eq!(
            st.header_walk.tip_header.map(|h| h.block_hash()),
            Some(h3.block_hash()),
            "restart keeps the tip header for the next look-ahead"
        );

        let base_h = st.header_walk.base_height;
        st.header_walk = HeaderWalk::default();
        hub.query.clear_milestone_path_above(0);
        st.height_to_hash
            .insert(base_h, BlockHash::from_byte_array([0x11; 32]));
        assert!(
            !restore_adopt(&mut st, &hub),
            "a base hash that is not on this chain is not restored"
        );
        assert!(!skips(&hub, 1, h1.block_hash().as_byte_array()));

        st.height_to_hash.remove(&base_h);
        assert!(restore_adopt(&mut st, &hub));
        assert!(st.header_walk.base_height > 0);
        let path = hub.query.store().path().join("header.adopt");
        on_confirmed_rewind(&mut st, &hub, 0);
        assert!(
            !path.exists(),
            "a reorg under the base deletes header.adopt"
        );
        hub.query.clear_milestone_path_above(0);
        assert!(!skips(&hub, 1, h1.block_hash().as_byte_array()));
        assert!(!restore_adopt(&mut st, &hub));

        st.header_walk = HeaderWalk::default();
        hub.query.clear_milestone_path_above(0);
        let path = hub.query.store().path().join("header.adopt");
        std::fs::write(&path, b"not-a-checkpoint-file").unwrap();
        assert!(!restore_adopt(&mut st, &hub));
        assert!(
            !skips(&hub, 1, h1.block_hash().as_byte_array()),
            "a file that does not parse does not skip scripts by height"
        );
    }

    #[test]
    fn header_request_prefers_the_peer_with_fewer_blocks_in_flight() {
        let (_dir, hub) = crate::chain::tiny_regtest_hub_labeled("header-walk-7");
        hub.ensure_genesis().unwrap();
        let gen = hub.tip_hash().unwrap();
        let (mut busy, mut busy_rx) = slot(0);
        let (quiet, mut quiet_rx) = slot(1);
        busy.in_flight.insert(BlockHash::from_byte_array([9u8; 32]));
        let mut st = IbdWorkState::new(vec![busy, quiet], Some(gen), Some(0));
        assert!(send_getheaders(&mut st, &hub).unwrap());
        assert!(
            busy_rx.try_recv().is_err(),
            "the busy peer is not asked first"
        );
        match quiet_rx.try_recv() {
            Ok(PeerCmd::GetHeaders { .. }) => {}
            Ok(_) => panic!("expected getheaders"),
            Err(_) => panic!("the quiet peer was not asked for headers"),
        }
        let block = BlockHash::from_byte_array([7u8; 32]);
        let mut room = 8usize;
        let mut issued = 0u64;
        assert!(crate::ibd::assign::issue_batch(
            &mut st,
            1,
            vec![block],
            &mut room,
            &mut issued
        ));
        match quiet_rx.try_recv() {
            Ok(PeerCmd::GetData { hashes }) => assert_eq!(hashes, vec![block]),
            Ok(_) => panic!("expected getdata"),
            Err(_) => panic!("the headers peer was not offered a block"),
        }
    }

    #[test]
    fn idle_header_requests_rotate_and_skip_a_short_peer() {
        let (_dir, hub) = crate::chain::tiny_regtest_hub_labeled("header-walk-rotate");
        hub.ensure_genesis().unwrap();
        let gen = hub.tip_hash().unwrap();
        let (a, mut rx_a) = slot(0);
        let (b, mut rx_b) = slot(1);
        let (mut short, mut rx_short) = slot(2);
        short.peer_height = 10;
        let mut st = IbdWorkState::new(vec![a, b, short], Some(gen), Some(0));
        st.header_walk.tip_height = 100;
        st.ordered.clear();
        for _ in 0..ORDERED_HEADERS_SOFT_CAP {
            push_dummy(&mut st, 0);
        }
        let mut got = [0u32; 3];
        for _ in 0..6 {
            assert!(send_getheaders(&mut st, &hub).unwrap());
            if rx_a.try_recv().is_ok() {
                got[0] += 1;
            }
            if rx_b.try_recv().is_ok() {
                got[1] += 1;
            }
            if rx_short.try_recv().is_ok() {
                got[2] += 1;
            }
            if let Some(peer) = st.header_walk.asked_peer {
                let _ = take_header_ask(&mut st, peer);
            }
        }
        assert_eq!(got[2], 0, "a peer at or below the walk tip is not asked");
        assert_eq!(got[0], 3, "idle peers share header requests");
        assert_eq!(got[1], 3, "idle peers share header requests");
    }

    #[test]
    fn off_path_headers_disconnect_the_peer_and_path_headers_do_not() {
        let (_dir, mut hub) = crate::chain::tiny_regtest_hub_labeled("header-walk-8");
        hub.ensure_genesis().unwrap();
        let gen = hub.tip_hash().unwrap();
        let h1 = mine(gen, 1);
        let h2 = mine(h1.block_hash(), 2);
        hub.milestone = rbitcoin_consensus::Milestone {
            height: 840_000,
            anchor: Some(rbitcoin_consensus::MilestoneAnchor {
                hash: BlockHash::from_byte_array([0xab; 32]),
                min_work_be: [0xff; 32],
            }),
        };
        let (path_peer, _rx0) = slot(0);
        let (side_peer, _rx1) = slot(1);
        let path_addr = path_peer.addr;
        let side_addr = side_peer.addr;
        let mut st = IbdWorkState::new(vec![path_peer, side_peer], Some(gen), Some(0));
        apply(&mut st, &hub, 0, vec![h1, h2]);
        assert!(st.slots.iter().any(|s| s.id == 0 && s.alive));
        assert!(!st.addr_cooldown.contains_key(&path_addr));

        let mut prev = gen;
        let mut side = Vec::with_capacity(4_001);
        for n in 0..4_001 {
            let hdr = mine(prev, 10_000 + n);
            prev = hdr.block_hash();
            side.push(hdr);
        }
        apply(&mut st, &hub, 1, side);
        assert!(
            st.slots.iter().any(|s| s.id == 1 && !s.alive),
            "a peer over the off-path budget is disconnected"
        );
        assert!(st.addr_cooldown.contains_key(&side_addr));
        assert!(st.slots.iter().any(|s| s.id == 0 && s.alive));
    }

    fn push_dummy(st: &mut IbdWorkState, n: u32) {
        let mut b = [0x11; 32];
        b[28..32].copy_from_slice(&n.to_le_bytes());
        let h = BlockHash::from_byte_array(b);
        st.ordered.push_back(h);
        st.ordered_set.insert(h);
    }

    /// The work floor is above anything a regtest header can reach.
    fn floor_unreachable(hub: &mut ChainHub) {
        hub.milestone = rbitcoin_consensus::Milestone {
            height: 840_000,
            anchor: Some(rbitcoin_consensus::MilestoneAnchor {
                hash: BlockHash::from_byte_array([0xab; 32]),
                min_work_be: [0xff; 32],
            }),
        };
    }

    fn queue_at_cap_ending_on(st: &mut IbdWorkState, hub: &ChainHub, tail: Header) {
        let mut n = 0u32;
        while st.ordered.len() + 1 < ORDERED_HEADERS_SOFT_CAP {
            push_dummy(st, n);
            n += 1;
        }
        apply(st, hub, 0, vec![tail]);
    }

    #[test]
    fn queue_refill_below_the_floor_stores_the_successor() {
        let (_dir, mut hub) = crate::chain::tiny_regtest_hub_labeled("header-walk-refill");
        hub.ensure_genesis().unwrap();
        let gen = hub.tip_hash().unwrap();
        let a1 = mine(gen, 1);
        let a2 = mine(a1.block_hash(), 2);
        let a3 = mine(a2.block_hash(), 3);
        floor_unreachable(&mut hub);
        let (s0, _rx0) = slot(0);
        let mut st = IbdWorkState::new(vec![s0], Some(gen), Some(0));
        queue_at_cap_ending_on(&mut st, &hub, a1);
        assert_eq!(st.ordered.back().copied(), Some(a1.block_hash()));
        let before = hub.query.store().header_count();
        assert!(send_getheaders(&mut st, &hub).unwrap());
        apply(&mut st, &hub, 0, vec![a2]);
        assert_eq!(hub.query.store().header_count(), before);
        assert_eq!(st.header_walk.tip_hash, Some(a2.block_hash()));

        let dropped = st.ordered.pop_front().unwrap();
        st.ordered_set.remove(&dropped);
        let dropped = st.ordered.pop_front().unwrap();
        st.ordered_set.remove(&dropped);
        apply(&mut st, &hub, 0, vec![a2, a3]);
        assert_eq!(
            hub.query.store().header_count(),
            before + 2,
            "the queue stores the successor of its tail below the work floor"
        );
        assert!(st.ordered_set.contains(&a2.block_hash()));
        assert!(st.ordered_set.contains(&a3.block_hash()));
    }

    #[test]
    fn easy_bits_lookahead_does_not_capture_the_walk() {
        let (_dir, hub) = crate::chain::tiny_regtest_hub_labeled("header-walk-easy");
        hub.ensure_genesis().unwrap();
        let gen = hub.tip_hash().unwrap();
        let honest = mine(gen, 1);
        let mut easy = honest;
        easy.bits = CompactTarget::from_consensus(0x2100ffff);
        let (s0, _rx0) = slot(0);
        let (s1, _rx1) = slot(1);
        let bad_addr = s0.addr;
        let mut st = IbdWorkState::new(vec![s0, s1], Some(gen), Some(0));
        fill_queue(&mut st);
        let before = hub.query.store().header_count();
        assert!(send_getheaders(&mut st, &hub).unwrap());
        apply(&mut st, &hub, 0, vec![easy]);
        assert_eq!(hub.query.store().header_count(), before);
        assert_eq!(st.header_walk.tip_hash, Some(gen));
        assert!(st.header_walk.checkpoints().is_empty());
        assert!(
            !st.slots[0].alive,
            "the peer that sent the easy header is down"
        );
        assert!(st.addr_cooldown.contains_key(&bad_addr));
        assert!(st.slots[1].alive);

        assert!(send_getheaders(&mut st, &hub).unwrap());
        apply(&mut st, &hub, 1, vec![honest]);
        assert_eq!(st.header_walk.tip_hash, Some(honest.block_hash()));
        assert_eq!(hub.query.store().header_count(), before);
        assert!(st.slots[1].alive);
    }

    #[test]
    fn unsolicited_lookahead_from_another_peer_is_ignored() {
        let (_dir, hub) = crate::chain::tiny_regtest_hub_labeled("header-walk-unasked");
        hub.ensure_genesis().unwrap();
        let gen = hub.tip_hash().unwrap();
        let good = mine(gen, 1);
        let (s0, _rx0) = slot(0);
        let (s1, _rx1) = slot(1);
        let mut st = IbdWorkState::new(vec![s0, s1], Some(gen), Some(0));
        fill_queue(&mut st);
        assert!(send_getheaders(&mut st, &hub).unwrap());
        apply(&mut st, &hub, 1, vec![good]);
        assert_eq!(st.header_walk.tip_hash, Some(gen));
        assert!(st.header_walk.checkpoints().is_empty());
        assert!(st.slots[1].alive, "an unasked peer is not disconnected");
        apply(&mut st, &hub, 0, vec![good]);
        assert_eq!(st.header_walk.tip_hash, Some(good.block_hash()));
        assert!(st.slots[1].alive);
    }

    #[test]
    fn rewind_clears_a_milestone_hash_from_the_abandoned_chain() {
        let (_dir, mut hub) = crate::chain::tiny_regtest_hub_labeled("header-walk-milestone");
        hub.ensure_genesis().unwrap();
        let gen = hub.tip_hash().unwrap();
        let a1 = mine(gen, 1);
        let a2 = mine(a1.block_hash(), 2);
        let b1 = mine(gen, 11);
        let b2 = mine(b1.block_hash(), 12);
        floor_unreachable(&mut hub);
        hub.milestone.height = 2;
        hub.milestone.anchor = Some(rbitcoin_consensus::MilestoneAnchor {
            hash: b2.block_hash(),
            min_work_be: [0xff; 32],
        });
        let (s0, _rx0) = slot(0);
        let (s1, _rx1) = slot(1);
        let mut st = IbdWorkState::new(vec![s0, s1], Some(gen), Some(0));
        fill_queue(&mut st);
        assert!(send_getheaders(&mut st, &hub).unwrap());
        apply(&mut st, &hub, 0, vec![a1, a2]);
        assert_eq!(st.header_walk.milestone_hash, Some(a2.block_hash()));
        let side = BlockHash::from_byte_array([0x42; 32]);
        assert!(!note_off_path(&mut st, &hub, 0, side));
        assert!(!note_off_path(&mut st, &hub, 0, side));
        assert_eq!(st.header_walk.off_path.get(&0).map(|s| s.len()), Some(1));
        plant_on_queue(&mut st, a1.block_hash(), 1);
        plant_on_queue(&mut st, a2.block_hash(), 2);

        apply(&mut st, &hub, 0, vec![]);
        apply(&mut st, &hub, 1, vec![]);
        assert_eq!(st.header_walk.tip_hash, Some(gen));
        assert!(
            st.header_walk.off_path.is_empty(),
            "a rewind forgets side headers from the abandoned chain"
        );
        assert!(
            st.header_walk.milestone_hash.is_none(),
            "rewinding under the milestone drops the abandoned hash"
        );

        apply(&mut st, &hub, 1, vec![b1, b2]);
        assert_eq!(st.header_walk.milestone_hash, Some(b2.block_hash()));
    }

    #[test]
    fn confirmed_reorg_drops_a_milestone_hash_below_the_fork() {
        let (_dir, mut hub) = crate::chain::tiny_regtest_hub_labeled("header-walk-reorg-ms");
        hub.ensure_genesis().unwrap();
        let gen = hub.tip_hash().unwrap();
        let a1 = mine(gen, 1);
        let a2 = mine(a1.block_hash(), 2);
        floor_unreachable(&mut hub);
        hub.milestone.height = 2;
        hub.milestone.anchor = Some(rbitcoin_consensus::MilestoneAnchor {
            hash: a2.block_hash(),
            min_work_be: [0xff; 32],
        });
        let (s0, _rx0) = slot(0);
        let mut st = IbdWorkState::new(vec![s0], Some(gen), Some(0));
        fill_queue(&mut st);
        assert!(send_getheaders(&mut st, &hub).unwrap());
        apply(&mut st, &hub, 0, vec![a1, a2]);
        assert_eq!(st.header_walk.base_height, 0);
        assert_eq!(st.header_walk.milestone_hash, Some(a2.block_hash()));

        hub.query.clear_milestone_path_above(2);
        on_confirmed_rewind(&mut st, &hub, 2);
        assert_eq!(st.header_walk.milestone_hash, Some(a2.block_hash()));
        assert_eq!(
            hub.query.milestone_header_at(2),
            Some(a2.block_hash().to_byte_array())
        );

        hub.query.clear_milestone_path_above(0);
        on_confirmed_rewind(&mut st, &hub, 0);
        assert!(
            st.header_walk.milestone_hash.is_none(),
            "a fork under the milestone lets the next chain record its hash"
        );
        st.header_walk = HeaderWalk::default();
        assert!(restore_adopt(&mut st, &hub));
        assert!(
            st.header_walk.milestone_hash.is_none(),
            "header.adopt does not restore a hash from below the fork"
        );
    }

    #[test]
    fn origin_stops_at_the_queue_tail() {
        let (_dir, mut hub) = crate::chain::tiny_regtest_hub_labeled("header-walk-origin");
        hub.ensure_genesis().unwrap();
        let gen = hub.tip_hash().unwrap();
        let h1 = mine(gen, 1);
        let h2 = mine(h1.block_hash(), 2);
        let h3 = mine(h2.block_hash(), 3);
        let next = mine(h2.block_hash(), 4);
        floor_unreachable(&mut hub);
        let (s0, _rx0) = slot(0);
        let mut st = IbdWorkState::new(vec![s0], Some(gen), Some(0));
        apply(&mut st, &hub, 0, vec![h1, h2, h3]);
        fill_to_cap(&mut st);
        plant_on_queue(&mut st, h2.block_hash(), 2);
        assert!(send_getheaders(&mut st, &hub).unwrap());
        apply(&mut st, &hub, 0, vec![next]);
        assert_eq!(st.header_walk.tip_hash, Some(next.block_hash()));
        assert_eq!(st.header_walk.base_height, 2);
    }

    fn mine_at(prev: BlockHash, n: u32, time: u32) -> Header {
        let mut h = Header {
            version: bitcoin::block::Version::from_consensus(4),
            prev_blockhash: prev,
            merkle_root: TxMerkleNode::from_byte_array([n as u8; 32]),
            time,
            bits: CompactTarget::from_consensus(0x207fffff),
            nonce: n,
        };
        rbitcoin_consensus::grind_regtest_pow(&mut h);
        h
    }

    #[test]
    fn refill_matching_the_first_of_two_checkpoints_is_stored() {
        let (_dir, mut hub) = crate::chain::tiny_regtest_hub_labeled("header-walk-two-ckpt");
        hub.ensure_genesis().unwrap();
        let gen = hub.tip_hash().unwrap();
        let a1 = mine(gen, 1);
        let a2 = mine(a1.block_hash(), 2);
        let a3 = mine(a2.block_hash(), 3);
        let a4 = mine(a3.block_hash(), 4);
        floor_unreachable(&mut hub);
        let (s0, _rx0) = slot(0);
        let mut st = IbdWorkState::new(vec![s0], Some(gen), Some(0));
        queue_at_cap_ending_on(&mut st, &hub, a1);
        assert!(send_getheaders(&mut st, &hub).unwrap());
        apply(&mut st, &hub, 0, vec![a2, a3]);
        assert!(send_getheaders(&mut st, &hub).unwrap());
        apply(&mut st, &hub, 0, vec![a4]);
        assert!(
            st.slots[0].alive,
            "the peer that extended the walk stays up"
        );
        assert_eq!(st.header_walk.checkpoints().len(), 2);
        let dropped = st.ordered.pop_front().unwrap();
        st.ordered_set.remove(&dropped);
        let before = hub.query.store().header_count();
        apply(&mut st, &hub, 0, vec![a2, a3]);
        assert_eq!(
            hub.query.store().header_count(),
            before + 2,
            "a refill that matches a checkpoint and stops before the next is stored"
        );
        assert!(hub
            .query
            .get_header_by_hash(a3.block_hash().as_byte_array())
            .unwrap()
            .is_some());
        assert_eq!(st.header_walk.checkpoints().len(), 2);
    }

    #[test]
    fn refill_that_contradicts_a_checkpoint_rewinds_and_stores() {
        let (_dir, mut hub) = crate::chain::tiny_regtest_hub_labeled("header-walk-rewind-miss");
        hub.ensure_genesis().unwrap();
        let gen = hub.tip_hash().unwrap();
        let a1 = mine(gen, 1);
        let a2 = mine(a1.block_hash(), 2);
        let honest = mine(a1.block_hash(), 9);
        floor_unreachable(&mut hub);
        let (s0, _rx0) = slot(0);
        let mut st = IbdWorkState::new(vec![s0], Some(gen), Some(0));
        queue_at_cap_ending_on(&mut st, &hub, a1);
        assert!(send_getheaders(&mut st, &hub).unwrap());
        apply(&mut st, &hub, 0, vec![a2]);
        assert_eq!(st.header_walk.tip_hash, Some(a2.block_hash()));
        let dropped = st.ordered.pop_front().unwrap();
        st.ordered_set.remove(&dropped);
        let before = hub.query.store().header_count();
        apply(&mut st, &hub, 0, vec![honest]);
        assert_eq!(hub.query.store().header_count(), before + 1);
        assert_eq!(st.header_walk.tip_hash, Some(honest.block_hash()));
        assert!(st
            .header_walk
            .checkpoints()
            .iter()
            .all(|c| c.hash != a2.block_hash()));
        assert!(!st.header_walk.dead_ends.contains(&honest.block_hash()));
        assert!(hub
            .query
            .get_header_by_hash(honest.block_hash().as_byte_array())
            .unwrap()
            .is_some());
        assert!(st.slots[0].alive);
    }

    #[test]
    fn a_second_lookahead_ask_waits_for_the_reply() {
        let (_dir, hub) = crate::chain::tiny_regtest_hub_labeled("header-walk-latch");
        hub.ensure_genesis().unwrap();
        let gen = hub.tip_hash().unwrap();
        let good = mine(gen, 1);
        let (s0, mut rx) = slot(0);
        let mut st = IbdWorkState::new(vec![s0], Some(gen), Some(0));
        fill_queue(&mut st);
        assert!(send_getheaders(&mut st, &hub).unwrap());
        assert!(matches!(rx.try_recv(), Ok(PeerCmd::GetHeaders { .. })));
        assert!(send_getheaders(&mut st, &hub).unwrap());
        assert!(
            rx.try_recv().is_err(),
            "a look-ahead ask still inside the window is not sent again"
        );
        apply(&mut st, &hub, 0, vec![good]);
        assert_eq!(st.header_walk.tip_hash, Some(good.block_hash()));
        assert!(st.slots[0].alive);
    }

    #[test]
    fn heavier_chain_below_the_floor_is_adopted_without_an_empty_reply() {
        let (_dir, mut hub) = crate::chain::tiny_regtest_hub_labeled("header-walk-heavy-floor");
        hub.ensure_genesis().unwrap();
        let gen = hub.tip_hash().unwrap();
        let stale = mine(gen, 1);
        floor_unreachable(&mut hub);
        let (s0, _rx0) = slot(0);
        let mut st = IbdWorkState::new(vec![s0], Some(gen), Some(0));
        fill_queue(&mut st);
        assert!(send_getheaders(&mut st, &hub).unwrap());
        apply(&mut st, &hub, 0, vec![stale]);
        assert_eq!(st.header_walk.tip_hash, Some(stale.block_hash()));
        let mut prev = gen;
        let mut honest = Vec::new();
        for n in 10..14 {
            let hdr = mine(prev, n);
            prev = hdr.block_hash();
            honest.push(hdr);
        }
        let tip = honest.last().unwrap().block_hash();
        apply(&mut st, &hub, 0, honest);
        assert_eq!(st.header_walk.tip_hash, Some(tip));
        assert!(st.slots[0].alive);
        assert!(st
            .header_walk
            .checkpoints()
            .iter()
            .all(|c| c.hash != stale.block_hash()));
    }

    #[test]
    fn a_short_peer_does_not_block_an_empty_rewind() {
        let (_dir, mut hub) = crate::chain::tiny_regtest_hub_labeled("header-walk-short-empty");
        hub.ensure_genesis().unwrap();
        let gen = hub.tip_hash().unwrap();
        let a1 = mine(gen, 1);
        let a2 = mine(a1.block_hash(), 2);
        floor_unreachable(&mut hub);
        let (tall, _rx0) = slot(0);
        let (mut short, _rx1) = slot(1);
        short.peer_height = 1;
        let mut st = IbdWorkState::new(vec![tall, short], Some(gen), Some(0));
        fill_queue(&mut st);
        assert!(send_getheaders(&mut st, &hub).unwrap());
        apply(&mut st, &hub, 0, vec![a1, a2]);
        assert_eq!(st.header_walk.tip_height, 2);
        plant_on_queue(&mut st, a1.block_hash(), 1);
        plant_on_queue(&mut st, a2.block_hash(), 2);
        apply(&mut st, &hub, 0, vec![]);
        assert_eq!(st.header_walk.tip_hash, Some(gen));
        assert!(st.slots.iter().any(|s| s.id == 1 && s.alive));
    }

    #[test]
    fn a_queued_hash_between_checkpoints_is_not_stored_while_the_queue_is_full() {
        let (_dir, mut hub) = crate::chain::tiny_regtest_hub_labeled("header-walk-between");
        hub.ensure_genesis().unwrap();
        let gen = hub.tip_hash().unwrap();
        let a1 = mine(gen, 1);
        floor_unreachable(&mut hub);
        let (s0, _rx0) = slot(0);
        let mut st = IbdWorkState::new(vec![s0], Some(gen), Some(0));
        fill_queue(&mut st);
        assert!(send_getheaders(&mut st, &hub).unwrap());
        apply(&mut st, &hub, 0, vec![a1]);
        let tip = st.header_walk.tip_hash;
        let before = hub.query.store().header_count();
        let mid = st.ordered[st.ordered.len() / 2];
        let side = mine(mid, 40);
        apply(&mut st, &hub, 0, vec![side]);
        assert_eq!(hub.query.store().header_count(), before);
        assert_eq!(st.header_walk.tip_hash, tip);
        assert!(!st.ordered_set.contains(&side.block_hash()));
    }

    #[test]
    fn restored_checkpoint_times_accept_a_header_under_the_tip_time() {
        let (_dir, hub) = crate::chain::tiny_regtest_hub_labeled("header-walk-mtp");
        hub.ensure_genesis().unwrap();
        let gen = hub.tip_hash().unwrap();
        let mut prev = gen;
        let mut chain = Vec::new();
        for n in 1..=12 {
            let hdr = mine(prev, n);
            prev = hdr.block_hash();
            chain.push(hdr);
        }
        let (s0, _rx0) = slot(0);
        let mut st = IbdWorkState::new(vec![s0], Some(gen), Some(0));
        fill_queue(&mut st);
        assert!(send_getheaders(&mut st, &hub).unwrap());
        apply(&mut st, &hub, 0, chain.clone());
        assert_eq!(st.header_walk.tip_times.len(), 11);
        let tip = chain[11];
        st.header_walk = HeaderWalk::default();
        assert!(restore_adopt(&mut st, &hub));
        assert_eq!(st.header_walk.tip_times.len(), 11);
        assert_eq!(
            st.header_walk.tip_header.map(|h| h.block_hash()),
            Some(tip.block_hash())
        );
        let soft = mine_at(tip.block_hash(), 13, tip.time - 2);
        assert!(send_getheaders(&mut st, &hub).unwrap());
        apply(&mut st, &hub, 0, vec![soft]);
        assert_eq!(st.header_walk.tip_hash, Some(soft.block_hash()));
        assert!(st.slots[0].alive);
    }

    #[test]
    fn a_missing_parent_header_is_consumed_without_a_disconnect() {
        let (_dir, mut hub) = crate::chain::tiny_regtest_hub_labeled("header-walk-no-parent");
        hub.ensure_genesis().unwrap();
        let gen = hub.tip_hash().unwrap();
        let stale = mine(gen, 1);
        floor_unreachable(&mut hub);
        let (s0, _rx0) = slot(0);
        let mut st = IbdWorkState::new(vec![s0], Some(gen), Some(0));
        fill_queue(&mut st);
        assert!(send_getheaders(&mut st, &hub).unwrap());
        apply(&mut st, &hub, 0, vec![stale]);
        st.header_walk.tip_header = None;
        st.header_walk.tip_times.clear();
        if let Some(c) = st.header_walk.checkpoints.last_mut() {
            c.header = None;
            c.times.clear();
        }
        let mut prev = stale.block_hash();
        let mut heavier = Vec::new();
        for n in 20..24 {
            let hdr = mine(prev, n);
            prev = hdr.block_hash();
            heavier.push(hdr);
        }
        let before = st.header_walk.tip_hash;
        apply(&mut st, &hub, 0, heavier);
        assert_eq!(st.header_walk.tip_hash, before);
        assert!(st.slots[0].alive);
    }

    #[test]
    fn min_difficulty_walk_back_rejects_a_limit_header() {
        let (_dir, mut hub) = crate::chain::tiny_regtest_hub_labeled("header-walk-mindiff");
        hub.ensure_genesis().unwrap();
        hub.params.btc.allow_min_difficulty_blocks = true;
        let gen = hub.tip_hash().unwrap();
        let tip = mine(gen, 1);
        let (s0, _rx0) = slot(0);
        let addr = s0.addr;
        let mut st = IbdWorkState::new(vec![s0], Some(gen), Some(0));
        fill_queue(&mut st);
        assert!(send_getheaders(&mut st, &hub).unwrap());
        apply(&mut st, &hub, 0, vec![tip]);
        assert!(st.slots[0].alive);
        st.header_walk.full_diff_bits = Some(CompactTarget::from_consensus(0x1d00ffff));
        st.header_walk.full_diff_height = st.header_walk.tip_height.saturating_sub(1);
        let next = mine_at(tip.block_hash(), 2, tip.time + 1);
        assert!(send_getheaders(&mut st, &hub).unwrap());
        apply(&mut st, &hub, 0, vec![next]);
        assert_eq!(st.header_walk.tip_hash, Some(tip.block_hash()));
        assert!(!st.slots[0].alive);
        assert!(st.addr_cooldown.contains_key(&addr));
    }

    #[test]
    fn a_restored_checkpoint_header_adopts_a_heavier_fork() {
        let (_dir, mut hub) = crate::chain::tiny_regtest_hub_labeled("header-walk-restore-fork");
        hub.ensure_genesis().unwrap();
        let gen = hub.tip_hash().unwrap();
        let fork = mine(gen, 1);
        let tip = mine(fork.block_hash(), 2);
        floor_unreachable(&mut hub);
        let (s0, _rx0) = slot(0);
        let mut st = IbdWorkState::new(vec![s0], Some(gen), Some(0));
        fill_queue(&mut st);
        assert!(send_getheaders(&mut st, &hub).unwrap());
        apply(&mut st, &hub, 0, vec![fork]);
        assert!(send_getheaders(&mut st, &hub).unwrap());
        apply(&mut st, &hub, 0, vec![tip]);
        assert_eq!(st.header_walk.checkpoints().len(), 2);
        st.header_walk = HeaderWalk::default();
        assert!(restore_adopt(&mut st, &hub));
        let mut prev = fork.block_hash();
        let mut heavier = Vec::new();
        for n in 30..34 {
            let hdr = mine(prev, n);
            prev = hdr.block_hash();
            heavier.push(hdr);
        }
        let heavy_tip = heavier.last().unwrap().block_hash();
        apply(&mut st, &hub, 0, heavier);
        assert_eq!(st.header_walk.tip_hash, Some(heavy_tip));
        assert!(st.slots[0].alive);
    }
}
