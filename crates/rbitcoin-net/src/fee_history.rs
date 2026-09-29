//! Per-height fee hurdles read from `txstat`, for historical fee estimates.
//!
//! Heights are kept newest-first up to a `txstat.body` byte budget. Each
//! height's hurdle feeds [`AnalogHistory`]; a height without one (a
//! coinbase-only block, or every tx below min relay) is held for the byte
//! and reorg bookkeeping but is not an observation.

use rbitcoin_mempool::{AnalogHistory, CONFIDENCE_FAR, CONFIDENCE_NEAR};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HistoricalFeeBlock {
    pub(crate) p10_sat_kvb: Option<u64>,
    pub(crate) txstat_bytes: u64,
}

#[derive(Debug)]
pub(crate) struct FeeHistory {
    blocks: BTreeMap<u32, HistoricalFeeBlock>,
    txstat_bytes: u64,
    budget: u64,
    targets: Vec<u32>,
    analog: AnalogHistory,
    /// An insert between held heights; rebuild before the next estimate.
    analog_stale: bool,
    /// Per-target rate, cleared on any change (one rebuild per new block).
    rates: Option<HashMap<u32, Option<u64>>>,
}

impl FeeHistory {
    pub(crate) fn new(budget: u64, targets: &[u32]) -> Self {
        Self {
            blocks: BTreeMap::new(),
            txstat_bytes: 0,
            budget,
            targets: targets.to_vec(),
            analog: AnalogHistory::new(targets),
            analog_stale: false,
            rates: None,
        }
    }

    pub(crate) fn get(&self, height: u32) -> Option<HistoricalFeeBlock> {
        self.blocks.get(&height).copied()
    }

    /// A connect at `height`: it replaces that height and every height above.
    pub(crate) fn insert(&mut self, height: u32, block: HistoricalFeeBlock) {
        self.truncate_above(height);
        if let Some(old) = self.blocks.remove(&height) {
            self.txstat_bytes = self.txstat_bytes.saturating_sub(old.txstat_bytes);
            if old.p10_sat_kvb.is_some() && !self.analog_stale {
                self.analog.pop_back();
            }
        }
        self.blocks.insert(height, block);
        self.txstat_bytes = self.txstat_bytes.saturating_add(block.txstat_bytes);
        if let (Some(rate), false) = (block.p10_sat_kvb, self.analog_stale) {
            self.analog.push_back(rate);
        }
        self.evict();
    }

    /// A preload row: kept only when the height is not held.
    pub(crate) fn insert_if_absent(&mut self, height: u32, block: HistoricalFeeBlock) {
        if self.blocks.contains_key(&height) {
            return;
        }
        self.rates = None;
        let first = self.blocks.first_key_value().map(|(&h, _)| h);
        let last = self.blocks.last_key_value().map(|(&h, _)| h);
        if let (Some(rate), false) = (block.p10_sat_kvb, self.analog_stale) {
            match (first, last) {
                (_, None) => self.analog.push_back(rate),
                (_, Some(last)) if height > last => self.analog.push_back(rate),
                (Some(first), _) if height < first => self.analog.push_front(rate),
                _ => self.analog_stale = true,
            }
        }
        self.blocks.insert(height, block);
        self.txstat_bytes = self.txstat_bytes.saturating_add(block.txstat_bytes);
        self.evict();
    }

    /// Drop every height above `height`.
    fn truncate_above(&mut self, height: u32) {
        self.rates = None;
        let above = self.blocks.split_off(&height.saturating_add(1));
        for old in above.values() {
            self.txstat_bytes = self.txstat_bytes.saturating_sub(old.txstat_bytes);
            if old.p10_sat_kvb.is_some() && !self.analog_stale {
                self.analog.pop_back();
            }
        }
    }

    fn evict(&mut self) {
        self.rates = None;
        while self.txstat_bytes > self.budget {
            let Some((_, old)) = self.blocks.pop_first() else {
                self.txstat_bytes = 0;
                break;
            };
            self.txstat_bytes = self.txstat_bytes.saturating_sub(old.txstat_bytes);
            if old.p10_sat_kvb.is_some() && !self.analog_stale {
                self.analog.pop_front();
            }
        }
    }

    /// Historical rate per target (1 block at [`CONFIDENCE_NEAR`], farther at
    /// [`CONFIDENCE_FAR`]); None while a target is not ready.
    pub(crate) fn rates(&mut self) -> HashMap<u32, Option<u64>> {
        if let Some(rates) = &self.rates {
            return rates.clone();
        }
        if self.analog_stale {
            self.analog
                .rebuild(self.blocks.values().filter_map(|b| b.p10_sat_kvb));
            self.analog_stale = false;
        }
        let rates: HashMap<u32, Option<u64>> = self
            .targets
            .iter()
            .map(|&n| {
                let confidence = if n <= 1 {
                    CONFIDENCE_NEAR
                } else {
                    CONFIDENCE_FAR
                };
                (n, self.analog.rate_sat_kvb(n, confidence))
            })
            .collect();
        self.rates = Some(rates.clone());
        rates
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rbitcoin_mempool::AnalogHistory;

    fn block(rate: Option<u64>, bytes: u64) -> HistoricalFeeBlock {
        HistoricalFeeBlock {
            p10_sat_kvb: rate,
            txstat_bytes: bytes,
        }
    }

    fn entries(history: &FeeHistory) -> Vec<(u32, HistoricalFeeBlock)> {
        history.blocks.iter().map(|(&h, &b)| (h, b)).collect()
    }

    fn calm(i: u32) -> u64 {
        1_000 + (u64::from(i) * 7_919) % 200
    }

    #[test]
    fn txstat_byte_budget_keeps_recent_heights_and_tracks_reorgs() {
        let mut history = FeeHistory::new(16, &[1]);
        for height in 1..=3 {
            history.insert(height, block(Some(u64::from(height) * 100), 8));
        }
        assert_eq!(
            entries(&history).iter().map(|e| e.0).collect::<Vec<_>>(),
            [2, 3]
        );
        assert_eq!(history.txstat_bytes, 16);

        history.insert(3, block(Some(350), 4));
        assert_eq!(history.txstat_bytes, 12);
        history.insert(2, block(Some(225), 8));
        assert_eq!(entries(&history), [(2, block(Some(225), 8))]);
        assert_eq!(history.txstat_bytes, 8);
    }

    #[test]
    fn heights_without_a_hurdle_are_not_observations() {
        let targets = [2];
        let mut with_empties = FeeHistory::new(u64::MAX, &targets);
        let mut hurdles_only = FeeHistory::new(u64::MAX, &targets);
        let mut next = 0;
        for height in 0..2_600u32 {
            if height % 10 == 9 {
                with_empties.insert(height, block(None, 8));
                continue;
            }
            with_empties.insert(height, block(Some(calm(height)), 8));
            hurdles_only.insert(next, block(Some(calm(height)), 8));
            next += 1;
        }
        assert_eq!(with_empties.analog.pairs(2), hurdles_only.analog.pairs(2));
        assert_eq!(with_empties.rates(), hurdles_only.rates());
        assert!(with_empties.rates()[&2].is_some());
    }

    #[test]
    fn every_update_path_matches_a_fresh_history() {
        let targets = [1, 2, 6];
        let mut history = FeeHistory::new(20_000, &targets);
        // preload walks down from 3000; connects arrive above it
        for height in (1_000..=3_000u32).rev() {
            history.insert_if_absent(height, block(Some(calm(height)), 8));
        }
        history.insert(3_001, block(Some(5_000), 8));
        history.insert(3_002, block(None, 8));
        // reorg: 3001 replaced, 3002 gone
        history.insert(3_001, block(Some(700), 8));
        // a gap fill between held heights marks the analog stale
        history.truncate_above(2_990);
        history.insert(2_995, block(Some(900), 8));
        history.insert_if_absent(2_993, block(Some(800), 8));
        assert!(history.analog_stale);
        // budget eviction pops the oldest
        for height in 2_996..3_600u32 {
            history.insert(height, block(Some(calm(height)), 8));
        }

        let mut fresh = FeeHistory::new(20_000, &targets);
        for (height, b) in entries(&history) {
            fresh.insert(height, b);
        }
        assert_eq!(history.rates(), fresh.rates());
        assert!(!history.analog_stale);
        let mut rebuilt = AnalogHistory::new(&targets);
        rebuilt.rebuild(entries(&history).iter().filter_map(|(_, b)| b.p10_sat_kvb));
        for n in targets {
            assert_eq!(history.analog.pairs(n), rebuilt.pairs(n), "N={n}");
        }
    }

    #[test]
    fn rates_are_kept_until_the_history_changes() {
        let mut history = FeeHistory::new(u64::MAX, &[1]);
        for height in 0..2_100 {
            history.insert(height, block(Some(calm(height)), 8));
        }
        let rates = history.rates();
        assert!(rates[&1].is_some());
        assert!(history.rates.is_some(), "cached");
        history.insert_if_absent(5, block(Some(1), 8));
        assert!(history.rates.is_some(), "held height changes nothing");
        history.insert(2_100, block(Some(1_000), 8));
        assert!(history.rates.is_none(), "a connect clears the cache");
    }
}
