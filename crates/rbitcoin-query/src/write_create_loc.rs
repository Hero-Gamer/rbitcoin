//! Write-thread loc pairs from Class A append (just-written parents).
//!
//! Lookup TipOnly reads `create.loc` after [`crate::Query::note_lookup_tiponly_start`].
//! A later pack whose TipOnly already started cannot have stamped those pairs.
//! Write keeps them until that last pack has finished write: `keep_until` is
//! `lookup_started_hi` at note (at least the noting pack height), then extended
//! while the pack is still at/above the load drain fence (TipOnly still skips
//! disk loc via InFlight). Prune when pack height is below that fence **and**
//! written height ≥ `keep_until`. The noting pack is not dropped on the same
//! write (`pack_height < written_hi`).
//!
//! This window is **write-thread TLS** (one writer per thread — same ownership
//! as load's [`crate::InFlight`], not a `Query` mutex). Disconnect is polled
//! via [`crate::Query::take_disconnect`] on the write thread.

use rbitcoin_primitives::Fk;
use rbitcoin_store::CreateLocPair;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use crate::Query;

const PAIR_BYTES: u64 = std::mem::size_of::<CreateLocPair>() as u64;

#[derive(Debug)]
struct LocPack {
    keep_until: u32,
    base: u64,
    pairs: Vec<CreateLocPair>,
    approx_bytes: u64,
}

impl LocPack {
    fn get(&self, id: u64) -> Option<CreateLocPair> {
        let off = id.checked_sub(self.base)?;
        self.pairs.get(off as usize).copied()
    }
}

/// Write-owned loc window. Note after append; prune after that write batch
/// (and later overlapping batches) finish.
#[derive(Debug, Default)]
pub(crate) struct WriteCreateLocRam {
    by_height: BTreeMap<u32, LocPack>,
    approx_bytes: u64,
}

impl WriteCreateLocRam {
    pub(crate) fn note(
        &mut self,
        pack_height: u32,
        keep_until: u32,
        fks: &[Fk],
        loc: &[CreateLocPair],
    ) {
        if fks.is_empty() || loc.len() != fks.len() {
            return;
        }
        let Some(start) = fks[0].get() else {
            return;
        };
        let bytes = (loc.len() as u64).saturating_mul(PAIR_BYTES);
        if let Some(old) = self.by_height.insert(
            pack_height,
            LocPack {
                keep_until,
                base: start,
                pairs: loc.to_vec(),
                approx_bytes: bytes,
            },
        ) {
            self.approx_bytes = self.approx_bytes.saturating_sub(old.approx_bytes);
        }
        self.approx_bytes = self.approx_bytes.saturating_add(bytes);
    }

    pub(crate) fn get(&self, fk: Fk) -> Option<CreateLocPair> {
        let id = fk.get()?;
        for pack in self.by_height.values() {
            if let Some(p) = pack.get(id) {
                return Some(p);
            }
        }
        None
    }

    /// Drop packs whose loc is on disk for later TipOnly (`pack_height < fence`)
    /// and whose overlapping lookup wave has finished write (`keep_until <=
    /// written_hi`). Keep the noting pack so a child that looked up while the
    /// parent was still in-flight can still fill abs.
    pub(crate) fn prune_written_through(
        &mut self,
        written_hi: u32,
        started_hi: Option<u32>,
        fence_hi: Option<u32>,
    ) {
        let started = started_hi.unwrap_or(written_hi);
        let fence = fence_hi.unwrap_or(written_hi);
        let drop: Vec<u32> = self
            .by_height
            .iter()
            .filter(|(h, p)| **h < fence && **h < written_hi && p.keep_until <= written_hi)
            .map(|(h, _)| *h)
            .collect();
        for h in drop {
            if let Some(p) = self.by_height.remove(&h) {
                self.approx_bytes = self.approx_bytes.saturating_sub(p.approx_bytes);
            }
        }
        for (h, pack) in self.by_height.iter_mut() {
            if *h >= fence {
                pack.keep_until = pack.keep_until.max(started);
            }
        }
    }

    /// Disconnect: drop packs at/above `height`. Remaining keep-until cannot
    /// wait for disconnected lookup batches.
    pub(crate) fn drop_from_height(&mut self, height: u32) {
        let drop = self.by_height.split_off(&height);
        for p in drop.into_values() {
            self.approx_bytes = self.approx_bytes.saturating_sub(p.approx_bytes);
        }
        let cap = height.saturating_sub(1);
        for p in self.by_height.values_mut() {
            if p.keep_until > cap {
                p.keep_until = cap;
            }
        }
    }

    /// Packs, pair count, approx bytes (IBD `wloc=`).
    pub(crate) fn size_snapshot(&self) -> (usize, usize, u64) {
        let pairs: usize = self.by_height.values().map(|p| p.pairs.len()).sum();
        (self.by_height.len(), pairs, self.approx_bytes)
    }
}

thread_local! {
    static RAM: RefCell<WriteCreateLocRam> = RefCell::new(WriteCreateLocRam::default());
    static DISCONNECT_SEEN: Cell<u64> = const { Cell::new(0) };
}

fn publish(ram: &WriteCreateLocRam) {
    let (packs, pairs, bytes) = ram.size_snapshot();
    crate::process_mem_stats::note_wloc(packs, pairs, bytes);
}

/// Drop this thread's loc window (Query open / test isolation). Does not publish
/// zeros — `wloc=` atomics are process-wide like `iflight=`.
pub(crate) fn clear() {
    RAM.with(|c| {
        *c.borrow_mut() = WriteCreateLocRam::default();
    });
    DISCONNECT_SEEN.with(|c| c.set(0));
}

pub(crate) fn with_ram<R>(query: &Query, f: impl FnOnce(&mut WriteCreateLocRam) -> R) -> R {
    RAM.with(|c| {
        let mut ram = c.borrow_mut();
        poll_disconnect(query, &mut ram);
        let r = f(&mut ram);
        publish(&ram);
        r
    })
}

fn poll_disconnect(query: &Query, ram: &mut WriteCreateLocRam) {
    let mut seen = DISCONNECT_SEEN.with(|c| c.get());
    if let Some(h) = query.take_disconnect(&mut seen) {
        ram.drop_from_height(h);
        DISCONNECT_SEEN.with(|c| c.set(seen));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rbitcoin_primitives::Fk;

    fn pair(n: u64) -> CreateLocPair {
        CreateLocPair {
            txout: (n * 10, 8),
            spent: (n * 9, 8),
            n_out: 1,
        }
    }

    fn fks(ids: &[u64]) -> Vec<Fk> {
        ids.iter().copied().map(Fk).collect()
    }

    fn loc(ids: &[u64]) -> Vec<CreateLocPair> {
        ids.iter().copied().map(pair).collect()
    }

    fn prune(m: &mut WriteCreateLocRam, written_hi: u32) {
        m.prune_written_through(written_hi, Some(written_hi), Some(written_hi));
    }

    #[test]
    fn note_get_by_fk() {
        let mut m = WriteCreateLocRam::default();
        m.note(1, 4, &fks(&[10, 11]), &loc(&[10, 11]));
        assert_eq!(m.get(Fk(10)).unwrap().txout, (100, 8));
        assert_eq!(m.get(Fk(11)).unwrap().spent, (99, 8));
        assert!(m.get(Fk(12)).is_none());
        assert!(m.get(Fk(9)).is_none());
    }

    #[test]
    fn prune_written_through_drops_at_keep_until() {
        let mut m = WriteCreateLocRam::default();
        m.note(1, 4, &fks(&[10]), &loc(&[10]));
        prune(&mut m, 3);
        assert!(m.get(Fk(10)).is_some(), "written_hi < keep_until keeps");
        prune(&mut m, 4);
        assert!(
            m.get(Fk(10)).is_none(),
            "written_hi == keep_until drops (last overlapping batch finished write)"
        );
        assert_eq!(m.size_snapshot(), (0, 0, 0));
    }

    #[test]
    fn prune_written_through_is_per_pack_keep_until() {
        let mut m = WriteCreateLocRam::default();
        m.note(1, 4, &fks(&[10]), &loc(&[10]));
        m.note(2, 8, &fks(&[11]), &loc(&[11]));
        prune(&mut m, 4);
        assert!(m.get(Fk(10)).is_none());
        assert!(m.get(Fk(11)).is_some(), "later pack waits for keep_until 8");
        prune(&mut m, 8);
        assert!(m.get(Fk(11)).is_none());
    }

    #[test]
    fn prune_keeps_noting_pack_until_later_write() {
        let mut m = WriteCreateLocRam::default();
        m.note(3, 3, &fks(&[10]), &loc(&[10]));
        prune(&mut m, 3);
        assert!(
            m.get(Fk(10)).is_some(),
            "same-write prune must not drop the noting pack (child TipOnly may still skip loc via InFlight)"
        );
        prune(&mut m, 4);
        assert!(m.get(Fk(10)).is_none());
        assert_eq!(m.size_snapshot(), (0, 0, 0));
    }

    #[test]
    fn prune_bumps_keep_until_from_started_hi_while_at_fence() {
        let mut m = WriteCreateLocRam::default();
        m.note(2, 1, &fks(&[10]), &loc(&[10]));
        m.prune_written_through(2, Some(8), Some(2));
        assert!(m.get(Fk(10)).is_some());
        m.prune_written_through(7, Some(8), Some(9));
        assert!(
            m.get(Fk(10)).is_some(),
            "keep_until bumped to 8 while pack was still at fence"
        );
        m.prune_written_through(8, Some(8), Some(9));
        assert!(m.get(Fk(10)).is_none());
    }

    #[test]
    fn no_count_cap_while_keep_until_open() {
        let mut m = WriteCreateLocRam::default();
        for i in 0..8u64 {
            m.note((i + 1) as u32, 100, &[Fk(1000 + i)], &[pair(1000 + i)]);
        }
        let (packs, pairs, bytes) = m.size_snapshot();
        assert_eq!(packs, 8);
        assert_eq!(pairs, 8);
        assert_eq!(bytes, 8 * PAIR_BYTES);
        assert!(m.get(Fk(1000)).is_some());
        assert!(m.get(Fk(1007)).is_some());
        prune(&mut m, 99);
        assert_eq!(m.size_snapshot().0, 8, "no silent FIFO drop");
        prune(&mut m, 100);
        assert_eq!(m.size_snapshot(), (0, 0, 0));
    }

    #[test]
    fn drop_from_height_drops_suffix_and_clamps() {
        let mut m = WriteCreateLocRam::default();
        m.note(5, 20, &fks(&[10]), &loc(&[10]));
        m.note(8, 20, &fks(&[11]), &loc(&[11]));
        m.drop_from_height(8);
        assert!(m.get(Fk(10)).is_some());
        assert!(m.get(Fk(11)).is_none());
        prune(&mut m, 7);
        assert!(
            m.get(Fk(10)).is_none(),
            "keep_until clamped to disconnect-1 so remaining pipeline can retire the pack"
        );
    }

    #[test]
    fn drop_from_height_zero_clears() {
        let mut m = WriteCreateLocRam::default();
        m.note(1, 4, &fks(&[10]), &loc(&[10]));
        m.drop_from_height(0);
        assert!(m.get(Fk(10)).is_none());
        assert_eq!(m.size_snapshot(), (0, 0, 0));
    }

    #[test]
    fn empty_or_len_mismatch_is_noop() {
        let mut m = WriteCreateLocRam::default();
        m.note(1, 4, &[], &[]);
        m.note(1, 4, &fks(&[10]), &loc(&[10, 11]));
        assert_eq!(m.size_snapshot(), (0, 0, 0));
        m.note(1, 4, &fks(&[0]), &loc(&[0]));
        assert!(m.get(Fk(0)).is_none(), "null fk does not note");
    }

    #[test]
    fn same_pack_height_replace_does_not_double_count_bytes() {
        let mut m = WriteCreateLocRam::default();
        m.note(1, 4, &fks(&[10, 11]), &loc(&[10, 11]));
        let (_, _, once) = m.size_snapshot();
        m.note(1, 8, &fks(&[10]), &loc(&[10]));
        let (packs, pairs, twice) = m.size_snapshot();
        assert_eq!(packs, 1);
        assert_eq!(pairs, 1);
        assert_eq!(twice, PAIR_BYTES);
        assert!(twice < once);
        assert!(m.get(Fk(11)).is_none());
        assert!(m.get(Fk(10)).is_some());
    }
}
