//! Missing-parent GETDATA tracker.
//!
//! One mutex owns the hash map and a per-peer time index. A heartbeat walks
//! only that peer's due entries. Caps are announcements, not a new knob.

use std::collections::{BTreeMap, HashMap, HashSet};

use super::{GETDATA_TX_INTERVAL_SECS, MAX_PARENTS_PER_PARK};

/// Announcements one peer may add. Past this the map does not grow.
pub(super) const MAX_PARENT_ANN_PER_PEER: usize = 5_000;
/// Process-wide announcements. A few hundred thousand, not tens of millions.
pub(super) const MAX_PARENT_ANN_GLOBAL: usize = 200_000;

/// One parent GETDATA the sweep should send.
pub(crate) struct DueParent {
    pub hash: [u8; 32],
    pub wtxid: bool,
}

/// Why `note_inv` did or did not record an announcement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ParentNote {
    Accepted,
    /// This peer's own counter is full.
    PeerCapped,
    /// The process-wide table is full. The announcement is skipped.
    GlobalFull,
}

struct ParentAnn {
    peer: u64,
    preferred: bool,
    reqtime: u64,
    /// Bucket in `due_by_peer` while this ann is waiting to be selected.
    due_at: Option<u64>,
    requested_until: Option<u64>,
    failed: bool,
    /// This announcement's getdata type. A wtxid inv does not retarget
    /// another peer's txid row.
    wtxid: bool,
}

struct ParentSlot {
    anns: Vec<ParentAnn>,
}

/// Hash map plus per-peer due and in-flight indexes. The indexes are one
/// entry per announcement and stop at the same caps (extra RAM, not a scan
/// of every peer on the heartbeat).
pub(super) struct ParentTracker {
    by_hash: HashMap<[u8; 32], ParentSlot>,
    due_by_peer: HashMap<u64, BTreeMap<u64, Vec<[u8; 32]>>>,
    inflight_by_peer: HashMap<u64, BTreeMap<u64, Vec<[u8; 32]>>>,
    hashes_by_peer: HashMap<u64, HashSet<[u8; 32]>>,
    ann_per_peer: HashMap<u64, usize>,
    announcements: usize,
}

impl ParentTracker {
    pub(super) fn new() -> Self {
        Self {
            by_hash: HashMap::new(),
            due_by_peer: HashMap::new(),
            inflight_by_peer: HashMap::new(),
            hashes_by_peer: HashMap::new(),
            ann_per_peer: HashMap::new(),
            announcements: 0,
        }
    }

    fn peer_at_cap(&self, peer: u64) -> bool {
        self.ann_per_peer.get(&peer).copied().unwrap_or(0) >= MAX_PARENT_ANN_PER_PEER
    }

    fn global_full(&self) -> bool {
        self.announcements >= MAX_PARENT_ANN_GLOBAL
    }

    pub(super) fn note_inv(
        &mut self,
        peer: u64,
        hash: [u8; 32],
        inbound: bool,
        now: u64,
        wtxid: bool,
    ) -> ParentNote {
        if self.rearm_existing(peer, hash, now, wtxid) {
            return ParentNote::Accepted;
        }
        if self.peer_at_cap(peer) {
            return ParentNote::PeerCapped;
        }
        if self.global_full() {
            return ParentNote::GlobalFull;
        }
        let exp = now.saturating_add(GETDATA_TX_INTERVAL_SECS);
        // Same-kind in-flight stays the one request. This announcer waits
        // until that window ends and keeps its own getdata type.
        let (due_at, requested_until) = match self.kind_inflight_until(&hash, wtxid) {
            Some(until) => (Some(until), None),
            None => (None, Some(exp)),
        };
        self.add_ann(
            hash,
            ParentAnn {
                peer,
                preferred: !inbound,
                reqtime: now,
                due_at,
                requested_until,
                failed: false,
                wtxid,
            },
        );
        ParentNote::Accepted
    }

    pub(super) fn schedule(&mut self, hash: [u8; 32], peer: u64, preferred: bool, reqtime: u64) {
        if self
            .by_hash
            .get(&hash)
            .is_some_and(|slot| slot.anns.iter().any(|a| a.peer == peer && !a.wtxid))
        {
            return;
        }
        if self.peer_at_cap(peer) || self.global_full() {
            return;
        }
        self.add_ann(
            hash,
            ParentAnn {
                peer,
                preferred,
                reqtime,
                due_at: Some(reqtime),
                requested_until: None,
                failed: false,
                wtxid: false,
            },
        );
    }

    /// True when this peer already had this kind of announcement for `hash`.
    fn rearm_existing(&mut self, peer: u64, hash: [u8; 32], now: u64, wtxid: bool) -> bool {
        let Some(slot) = self.by_hash.get_mut(&hash) else {
            return false;
        };
        let Some(pos) = slot
            .anns
            .iter()
            .position(|a| a.peer == peer && a.wtxid == wtxid)
        else {
            return false;
        };
        let due_at = {
            if slot.anns[pos].requested_until.is_some() || slot.anns[pos].failed {
                return true;
            }
            let exp = now.saturating_add(GETDATA_TX_INTERVAL_SECS);
            let due_at = slot.anns[pos].due_at;
            slot.anns[pos].requested_until = Some(exp);
            slot.anns[pos].due_at = None;
            (due_at, exp)
        };
        if let Some(t) = due_at.0 {
            unindex(&mut self.due_by_peer, peer, t, &hash);
        }
        index_at(&mut self.inflight_by_peer, peer, due_at.1, hash);
        true
    }

    fn add_ann(&mut self, hash: [u8; 32], ann: ParentAnn) {
        let peer = ann.peer;
        let due_at = ann.due_at;
        let exp = ann.requested_until;
        let slot = self
            .by_hash
            .entry(hash)
            .or_insert_with(|| ParentSlot { anns: Vec::new() });
        slot.anns.push(ann);
        *self.ann_per_peer.entry(peer).or_default() += 1;
        self.announcements += 1;
        self.hashes_by_peer.entry(peer).or_default().insert(hash);
        if let Some(exp) = exp {
            index_at(&mut self.inflight_by_peer, peer, exp, hash);
        } else if let Some(t) = due_at {
            index_at(&mut self.due_by_peer, peer, t, hash);
        }
    }

    pub(super) fn forget_peer(&mut self, peer: u64) {
        self.due_by_peer.remove(&peer);
        self.inflight_by_peer.remove(&peer);
        let Some(hashes) = self.hashes_by_peer.remove(&peer) else {
            return;
        };
        for hash in hashes {
            self.remove_peer_ann(&hash, peer);
        }
    }

    pub(super) fn announcer_peers(&self, hashes: [[u8; 32]; 2]) -> Vec<u64> {
        let mut peers = Vec::new();
        for hash in hashes {
            let Some(slot) = self.by_hash.get(&hash) else {
                continue;
            };
            for a in &slot.anns {
                if !a.failed && !peers.contains(&a.peer) {
                    peers.push(a.peer);
                }
            }
        }
        peers
    }

    pub(super) fn resolve(&mut self, hashes: [[u8; 32]; 2], admitted: bool) {
        for hash in hashes {
            if admitted {
                self.remove_hash(&hash);
                continue;
            }
            let inflight = self.inflight_peers(&hash);
            for (peer, exp) in inflight {
                self.fail_inflight(&hash, peer, exp);
            }
            if self.slot_all_failed(&hash) {
                self.remove_hash(&hash);
            }
        }
    }

    pub(super) fn take_due(
        &mut self,
        peer: u64,
        now: u64,
        mut already_have: impl FnMut(&[u8; 32], bool) -> bool,
    ) -> Vec<DueParent> {
        self.expire_peer_inflight(peer, now);
        let due_hashes = self.due_hashes(peer, now);
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for hash in due_hashes {
            if out.len() >= MAX_PARENTS_PER_PARK {
                break;
            }
            if !seen.insert(hash) {
                continue;
            }
            self.expire_hash_inflight(&hash, now);
            if !self.by_hash.contains_key(&hash) {
                continue;
            }
            if self.slot_all_failed(&hash) {
                self.remove_hash(&hash);
                continue;
            }
            let Some(wtxid) = self.select(peer, now, &hash) else {
                continue;
            };
            if already_have(&hash, wtxid) {
                self.drop_kind(&hash, wtxid);
                continue;
            }
            out.push(DueParent { hash, wtxid });
        }
        out
    }

    fn due_hashes(&self, peer: u64, now: u64) -> Vec<[u8; 32]> {
        let Some(tree) = self.due_by_peer.get(&peer) else {
            return Vec::new();
        };
        tree.range(..=now)
            .flat_map(|(_, v)| v.iter().copied())
            .collect()
    }

    /// When this kind is already in flight, the time that request ends.
    fn kind_inflight_until(&self, hash: &[u8; 32], wtxid: bool) -> Option<u64> {
        self.by_hash.get(hash).and_then(|slot| {
            slot.anns
                .iter()
                .filter(|a| a.wtxid == wtxid)
                .filter_map(|a| a.requested_until)
                .min()
        })
    }

    fn select(&mut self, peer: u64, now: u64, hash: &[u8; 32]) -> Option<bool> {
        let exp = now.saturating_add(GETDATA_TX_INTERVAL_SECS);
        let (due_at, wtxid) = {
            let slot = self.by_hash.get_mut(hash)?;
            let has_pref = slot
                .anns
                .iter()
                .any(|a| a.preferred && !a.failed && a.reqtime <= now);
            let pos = slot.anns.iter().position(|a| {
                !a.failed && a.reqtime <= now && a.peer == peer && (!has_pref || a.preferred)
            })?;
            let wtxid = slot.anns[pos].wtxid;
            // Any in-flight request for these bytes blocks a second getdata.
            // A non-segwit wtxid inv uses the txid, so the orphan parent is
            // already in flight. The kind we would send stays this ann's own.
            if slot.anns.iter().any(|a| a.requested_until.is_some()) {
                return None;
            }
            let ann = &mut slot.anns[pos];
            let due_at = ann.due_at.unwrap_or(ann.reqtime);
            ann.requested_until = Some(exp);
            ann.due_at = None;
            (due_at, wtxid)
        };
        unindex(&mut self.due_by_peer, peer, due_at, hash);
        index_at(&mut self.inflight_by_peer, peer, exp, *hash);
        Some(wtxid)
    }

    fn expire_peer_inflight(&mut self, peer: u64, now: u64) {
        let expired = self
            .inflight_by_peer
            .get(&peer)
            .map(|tree| {
                tree.range(..=now)
                    .flat_map(|(t, v)| v.iter().map(|h| (*t, *h)))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (exp, hash) in expired {
            self.fail_inflight(&hash, peer, exp);
            if self.slot_all_failed(&hash) {
                self.remove_hash(&hash);
            }
        }
    }

    fn expire_hash_inflight(&mut self, hash: &[u8; 32], now: u64) {
        let expired = self.inflight_peers(hash);
        for (peer, exp) in expired {
            if exp <= now {
                self.fail_inflight(hash, peer, exp);
            }
        }
        if self.slot_all_failed(hash) {
            self.remove_hash(hash);
        }
    }

    fn inflight_peers(&self, hash: &[u8; 32]) -> Vec<(u64, u64)> {
        self.by_hash
            .get(hash)
            .map(|slot| {
                slot.anns
                    .iter()
                    .filter_map(|a| a.requested_until.map(|exp| (a.peer, exp)))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn fail_inflight(&mut self, hash: &[u8; 32], peer: u64, exp: u64) {
        // Expire the ann whose window is `exp`. A same-peer txid parent is a
        // different row and must stay eligible once this window ends.
        if let Some(slot) = self.by_hash.get_mut(hash) {
            if let Some(ann) = slot
                .anns
                .iter_mut()
                .find(|a| a.peer == peer && a.requested_until == Some(exp))
            {
                ann.requested_until = None;
                ann.failed = true;
            }
        }
        unindex(&mut self.inflight_by_peer, peer, exp, hash);
    }

    fn slot_all_failed(&self, hash: &[u8; 32]) -> bool {
        self.by_hash
            .get(hash)
            .is_some_and(|slot| !slot.anns.is_empty() && slot.anns.iter().all(|a| a.failed))
    }

    fn remove_hash(&mut self, hash: &[u8; 32]) {
        let Some(slot) = self.by_hash.remove(hash) else {
            return;
        };
        for ann in slot.anns {
            self.note_removed_ann(&ann, hash);
        }
    }

    fn drop_kind(&mut self, hash: &[u8; 32], wtxid: bool) {
        let peers: Vec<u64> = self
            .by_hash
            .get(hash)
            .map(|slot| {
                slot.anns
                    .iter()
                    .filter(|a| a.wtxid == wtxid)
                    .map(|a| a.peer)
                    .collect()
            })
            .unwrap_or_default();
        for peer in peers {
            self.remove_peer_ann_kind(hash, peer, Some(wtxid));
        }
    }

    fn remove_peer_ann(&mut self, hash: &[u8; 32], peer: u64) {
        self.remove_peer_ann_kind(hash, peer, None);
    }

    fn remove_peer_ann_kind(&mut self, hash: &[u8; 32], peer: u64, kind: Option<bool>) {
        loop {
            let Some(ann) = self.by_hash.get_mut(hash).and_then(|slot| {
                let pos = slot
                    .anns
                    .iter()
                    .position(|a| a.peer == peer && kind.is_none_or(|k| a.wtxid == k))?;
                Some(slot.anns.swap_remove(pos))
            }) else {
                return;
            };
            if self
                .by_hash
                .get(hash)
                .is_some_and(|slot| slot.anns.is_empty())
            {
                self.by_hash.remove(hash);
            }
            self.note_removed_ann(&ann, hash);
            if kind.is_some() {
                return;
            }
        }
    }

    fn note_removed_ann(&mut self, ann: &ParentAnn, hash: &[u8; 32]) {
        self.announcements = self.announcements.saturating_sub(1);
        if let Some(c) = self.ann_per_peer.get_mut(&ann.peer) {
            *c = c.saturating_sub(1);
            if *c == 0 {
                self.ann_per_peer.remove(&ann.peer);
            }
        }
        if let Some(set) = self.hashes_by_peer.get_mut(&ann.peer) {
            let still = self
                .by_hash
                .get(hash)
                .is_some_and(|slot| slot.anns.iter().any(|a| a.peer == ann.peer));
            if !still {
                set.remove(hash);
                if set.is_empty() {
                    self.hashes_by_peer.remove(&ann.peer);
                }
            }
        }
        if let Some(exp) = ann.requested_until {
            unindex(&mut self.inflight_by_peer, ann.peer, exp, hash);
        }
        if let Some(t) = ann.due_at {
            unindex(&mut self.due_by_peer, ann.peer, t, hash);
        }
    }
}

fn index_at(
    tree: &mut HashMap<u64, BTreeMap<u64, Vec<[u8; 32]>>>,
    peer: u64,
    time: u64,
    hash: [u8; 32],
) {
    tree.entry(peer)
        .or_default()
        .entry(time)
        .or_default()
        .push(hash);
}

fn unindex(
    tree: &mut HashMap<u64, BTreeMap<u64, Vec<[u8; 32]>>>,
    peer: u64,
    time: u64,
    hash: &[u8; 32],
) {
    let Some(by_time) = tree.get_mut(&peer) else {
        return;
    };
    let Some(v) = by_time.get_mut(&time) else {
        return;
    };
    if let Some(i) = v.iter().position(|h| h == hash) {
        v.swap_remove(i);
    }
    if v.is_empty() {
        by_time.remove(&time);
    }
    if by_time.is_empty() {
        tree.remove(&peer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn txid_parent_already_have_does_not_follow_another_peers_wtxid() {
        let mut t = ParentTracker::new();
        let hash = [0xcd; 32];
        t.schedule(hash, 2, true, 1_000);
        assert_eq!(
            t.note_inv(1, hash, false, 1_000, true),
            ParentNote::Accepted
        );
        assert_eq!(
            t.note_inv(3, hash, false, 1_000, true),
            ParentNote::Accepted
        );
        let now = 1_000 + GETDATA_TX_INTERVAL_SECS;
        let mut saw_txid = false;
        let dropped = t.take_due(2, now, |h, wtxid| {
            assert_eq!(*h, hash);
            assert!(!wtxid, "txid parent is checked as a txid");
            saw_txid = true;
            true
        });
        assert!(saw_txid);
        assert!(
            dropped.is_empty(),
            "a txid parent already in the mempool is not requested"
        );
        let follow = t.take_due(3, now, |_, wtxid| {
            assert!(wtxid, "wtxid follow-up is not checked as a txid");
            false
        });
        assert_eq!(follow.len(), 1);
        assert!(follow[0].wtxid);
    }

    #[test]
    fn inflight_wtxid_of_the_same_hash_is_not_requested_again_as_txid() {
        let mut t = ParentTracker::new();
        let inflight = [0x11; 32];
        let missing = [0x22; 32];
        assert_eq!(
            t.note_inv(1, inflight, true, 1_000, true),
            ParentNote::Accepted
        );
        t.schedule(inflight, 2, false, 1_000);
        t.schedule(missing, 2, false, 1_000);
        let due = t.take_due(2, 1_000, |_, _| false);
        assert_eq!(
            due.len(),
            1,
            "the in-flight hash must not join this getdata"
        );
        assert_eq!(due[0].hash, missing);
        assert!(!due[0].wtxid, "the other parent stays a txid request");
    }
}
