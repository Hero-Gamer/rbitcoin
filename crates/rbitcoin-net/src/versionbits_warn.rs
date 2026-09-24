//! Unknown-versionbits activation warnings (Core `WarningBitsConditionChecker`).
//!
//! Regtest/testnets: period = difficulty interval, threshold = 75% of period.
//! When a completed period has ≥threshold blocks signalling an unassigned bit
//! with BIP9 top bits, the next tip reports
//! `Unknown new rules activated (versionbit N)`.

use rbitcoin_primitives::{Height, Network};
use rbitcoin_query::Query;
use std::sync::Mutex;

const VERSIONBITS_TOP_BITS: u32 = 0x2000_0000;
const VERSIONBITS_TOP_MASK: u32 = 0xe000_0000;
const VERSIONBITS_NUM_BITS: i32 = 29;

/// Period / threshold for unknown-bit warnings (Core test-chain rule).
pub fn warn_period_threshold(network: Network) -> (u32, u32) {
    let period = match network {
        Network::Regtest => 144,
        Network::Testnet | Network::Signet => 2016,
        Network::Mainnet => 2016,
    };
    let threshold = period * 3 / 4;
    (period, threshold)
}

/// Format Core's unknown-rules warning for `bit`.
pub fn unknown_rules_warning(bit: i32) -> String {
    format!("Unknown new rules activated (versionbit {bit})")
}

/// Unknown bits that reached ACTIVE on the best chain, kept across calls.
///
/// A bit locked in during period `p` is ACTIVE from period `p + 2`, and
/// ACTIVE is final, so each completed period is read once and only periods
/// completed since the last call are read. RAM is a few words; the trade is
/// one header pass per new period instead of 29 passes from genesis per RPC
/// call (≈4.7 s per `getnetworkinfo` at mainnet height 968k).
pub(crate) struct UnknownBitsScan {
    network: Option<Network>,
    /// Periods `0..scanned_periods` are counted.
    scanned_periods: u32,
    /// Hash of the last header of the last counted period (reorg check).
    boundary: [u8; 32],
    /// Bits that reached the threshold in a counted period.
    locked: u32,
}

impl UnknownBitsScan {
    const fn new() -> Self {
        Self {
            network: None,
            scanned_periods: 0,
            boundary: [0; 32],
            locked: 0,
        }
    }

    pub(crate) fn active_bits(&mut self, query: &Query, network: Network) -> Vec<i32> {
        if self.network != Some(network) {
            *self = Self::new();
            self.network = Some(network);
        }
        let Some(tip) = query.tip_height() else {
            return Vec::new();
        };
        let (period, threshold) = warn_period_threshold(network);
        self.advance(query, tip.0, period, threshold)
    }

    fn advance(&mut self, query: &Query, tip: u32, period: u32, threshold: u32) -> Vec<i32> {
        // The most recent completed period is at most LOCKED_IN, not ACTIVE.
        let target = (tip / period).saturating_sub(1);
        if target < self.scanned_periods || !self.boundary_matches(query, period) {
            self.scanned_periods = 0;
            self.boundary = [0; 32];
            self.locked = 0;
        }
        for p in self.scanned_periods..target {
            let mut counts = [0u32; VERSIONBITS_NUM_BITS as usize];
            for h in (p * period).max(1)..(p + 1) * period {
                if let Ok(Some((_, rec))) = query.header_at_height(Height(h)) {
                    for (bit, count) in counts.iter_mut().enumerate() {
                        if signals_unknown(&rec.version, bit as i32) {
                            *count += 1;
                        }
                    }
                }
            }
            for (bit, count) in counts.iter().enumerate() {
                if *count >= threshold {
                    self.locked |= 1 << bit;
                }
            }
        }
        if target > self.scanned_periods {
            self.scanned_periods = target;
            self.boundary = query
                .header_at_height(Height(target * period - 1))
                .ok()
                .flatten()
                .map_or([0; 32], |(_, rec)| rec.hash);
        }
        (0..VERSIONBITS_NUM_BITS)
            .filter(|bit| (self.locked >> bit) & 1 == 1)
            .collect()
    }

    fn boundary_matches(&self, query: &Query, period: u32) -> bool {
        if self.scanned_periods == 0 {
            return true;
        }
        let h = self.scanned_periods * period - 1;
        matches!(query.header_at_height(Height(h)), Ok(Some((_, rec))) if rec.hash == self.boundary)
    }
}

fn signals_unknown(version: &i32, bit: i32) -> bool {
    let v = *version as u32;
    (v & VERSIONBITS_TOP_MASK) == VERSIONBITS_TOP_BITS && ((v >> bit) & 1) != 0
}

/// Warning strings for RPC `warnings` arrays.
pub fn warning_strings(query: &Query, network: Network) -> Vec<String> {
    static SCAN: Mutex<UnknownBitsScan> = Mutex::new(UnknownBitsScan::new());
    SCAN.lock()
        .unwrap_or_else(|e| e.into_inner())
        .active_bits(query, network)
        .into_iter()
        .map(unknown_rules_warning)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regtest_period_matches_core_functional() {
        let (p, t) = warn_period_threshold(Network::Regtest);
        assert_eq!(p, 144);
        assert_eq!(t, 108);
    }

    #[test]
    fn warning_text_matches_core() {
        assert_eq!(
            unknown_rules_warning(27),
            "Unknown new rules activated (versionbit 27)"
        );
    }

    #[test]
    fn top_bits_signal_detection() {
        let v = (VERSIONBITS_TOP_BITS | (1 << 27)) as i32;
        assert!(signals_unknown(&v, 27));
        assert!(!signals_unknown(&v, 26));
        assert!(!signals_unknown(&(VERSIONBITS_TOP_BITS as i32), 27));
    }

    /// Regtest-shaped chain of `n` headers; heights in `signal` set bit 27.
    fn put_chain(q: &Query, n: u32, signal: impl Fn(u32) -> bool, salt: u8) -> Vec<[u8; 32]> {
        use rbitcoin_primitives::Fk;
        use rbitcoin_store::HeaderRecord;
        let mut prev_fk = Fk::NULL;
        let mut prev_hash = [0u8; 32];
        let mut hashes = Vec::new();
        for h in 0u32..n {
            let mut merkle = [salt; 32];
            merkle[0..4].copy_from_slice(&h.to_le_bytes());
            let version = if h > 0 && signal(h) {
                (VERSIONBITS_TOP_BITS | (1 << 27)) as i32
            } else {
                1
            };
            let hash = if h == 0 {
                merkle
            } else {
                rbitcoin_store::block_header_hash(
                    version,
                    &prev_hash,
                    &merkle,
                    h + 1,
                    0x207fffff,
                    h,
                )
            };
            let rec = HeaderRecord {
                prev_fk,
                version,
                timestamp: h + 1,
                bits: 0x207fffff,
                nonce: h,
                merkle_root: merkle,
                hash,
                size: 0,
                weight: 0,
            };
            prev_fk = q.put_header(&rec).unwrap();
            q.store().confirmed.set(Height(h), prev_fk).unwrap();
            prev_hash = hash;
            hashes.push(hash);
        }
        q.store().rebuild_height_fence().unwrap();
        hashes
    }

    /// `getnetworkinfo` / `getblockchaininfo` read every header from genesis
    /// for each of 29 bits (≈4.7 s on mainnet). The scan keeps its place and
    /// only reads periods completed since the last call.
    #[test]
    fn scan_reads_only_new_periods_and_restarts_after_a_reorg() {
        let (dir, q) = rbitcoin_query::testutil::tiny_query_labeled("vb-scan");
        let period = 144;
        put_chain(&q, 3 * period + 10, |h| h < period, 0);
        let mut scan = UnknownBitsScan::new();
        assert_eq!(scan.active_bits(&q, Network::Regtest), vec![27]);
        assert_eq!(scan.scanned_periods, 2);

        assert_eq!(scan.active_bits(&q, Network::Regtest), vec![27]);
        assert_eq!(scan.scanned_periods, 2, "same tip: nothing new to read");

        let (dir2, q2) = rbitcoin_query::testutil::tiny_query_labeled("vb-scan-reorg");
        put_chain(&q2, 3 * period + 10, |_| false, 1);
        assert!(
            scan.active_bits(&q2, Network::Regtest).is_empty(),
            "a different chain at the scanned boundary starts over"
        );
        assert_eq!(scan.scanned_periods, 2);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
    }

    #[test]
    fn unknown_bit_is_active_two_periods_after_threshold() {
        use rbitcoin_primitives::{Fk, Height};
        use rbitcoin_store::HeaderRecord;

        let (dir, q) = rbitcoin_query::testutil::tiny_query_labeled("vb-active");
        assert!(UnknownBitsScan::new().advance(&q, 0, 4, 2).is_empty());
        assert!(UnknownBitsScan::new().advance(&q, 4, 4, 2).is_empty());

        let signal = (VERSIONBITS_TOP_BITS | (1 << 27)) as i32;
        let mut prev_fk = Fk::NULL;
        let mut prev_hash = [0u8; 32];
        for h in 0u32..=8 {
            let mut merkle = [0u8; 32];
            merkle[0..4].copy_from_slice(&h.to_le_bytes());
            let version = if h == 0 || h >= 4 { 1 } else { signal };
            let timestamp = h + 1;
            let bits = 0x207fffff;
            let nonce = h;
            let hash = if h == 0 {
                merkle
            } else {
                rbitcoin_store::block_header_hash(
                    version, &prev_hash, &merkle, timestamp, bits, nonce,
                )
            };
            let rec = HeaderRecord {
                prev_fk,
                version,
                timestamp,
                bits,
                nonce,
                merkle_root: merkle,
                hash,
                size: 0,
                weight: 0,
            };
            prev_fk = q.put_header(&rec).unwrap();
            q.store().confirmed.set(Height(h), prev_fk).unwrap();
            prev_hash = hash;
        }
        q.store().rebuild_height_fence().unwrap();
        assert_eq!(UnknownBitsScan::new().advance(&q, 8, 4, 2), vec![27]);
        assert!(UnknownBitsScan::new().advance(&q, 8, 4, 4).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
