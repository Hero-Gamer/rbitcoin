//! Write-thread loc pairs from Class A append (just-written parents).
//!
//! Lookup TipOnly reads `create.loc` after [`crate::Query::note_lookup_tiponly_start`].
//! A later pack whose TipOnly already started cannot have stamped those pairs.
//! Write keeps them until that last pack has finished write: `keep_until` is
//! `lookup_started_hi` at note (at least the noting pack height). Prune when
//! written height ≥ `keep_until`. Disconnect [`Self::drop_from_height`] drops
//! packs at/above that height and clamps remaining `keep_until`. Height index
//! matches [`crate::InFlight`]; the drop gate is written height vs keep-until,
//! not drain+fence vs pack height.

use rbitcoin_primitives::Fk;
use rbitcoin_store::CreateLocPair;
use std::collections::BTreeMap;

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

    /// Drop packs whose last overlapping lookup batch has finished write.
    ///
    /// Equality drops (`keep_until == written_hi`).
    pub(crate) fn prune_written_through(&mut self, written_hi: u32) {
        let drop: Vec<u32> = self
            .by_height
            .iter()
            .filter(|(_, p)| p.keep_until <= written_hi)
            .map(|(h, _)| *h)
            .collect();
        for h in drop {
            if let Some(p) = self.by_height.remove(&h) {
                self.approx_bytes = self.approx_bytes.saturating_sub(p.approx_bytes);
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
        m.prune_written_through(3);
        assert!(m.get(Fk(10)).is_some(), "written_hi < keep_until keeps");
        m.prune_written_through(4);
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
        m.prune_written_through(4);
        assert!(m.get(Fk(10)).is_none());
        assert!(m.get(Fk(11)).is_some(), "later pack waits for keep_until 8");
        m.prune_written_through(8);
        assert!(m.get(Fk(11)).is_none());
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
        m.prune_written_through(99);
        assert_eq!(m.size_snapshot().0, 8, "no silent FIFO drop");
        m.prune_written_through(100);
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
        m.prune_written_through(7);
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
