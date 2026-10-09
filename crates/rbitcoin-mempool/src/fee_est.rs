//! Flow-aware fee projection (Engine v2 pure math).
//!
//! Capacity always uses a fixed 10-minute block clock:
//! `capacity_wu(N) = N × BLOCK_WEIGHT_WU`. Live stock + projected inflow above
//! a candidate rate decide the minimum inclusion feerate.

/// Consensus block weight (WU) used as one-block capacity.
pub const BLOCK_WEIGHT_WU: u64 = 4_000_000;

/// Seconds per planned block (product clock; not wall time since last tip).
pub const SECONDS_PER_BLOCK: u64 = 600;

/// Inclusion confidence at N=1 (10-minute default). Higher → higher sat/vB.
pub const CONFIDENCE_NEAR: f64 = 0.999;
/// Inclusion confidence for historical/blended targets from N=2 onward.
pub const CONFIDENCE_FAR: f64 = 0.99;
/// Fraction of `N×4e6` WU to fill at `CONFIDENCE_NEAR` (leave shock room).
pub const FILL_AT_NEAR: f64 = 0.80;
/// Fraction of `N×4e6` WU to fill at `CONFIDENCE_FAR` (today's 95% haircut).
pub const FILL_AT_FAR: f64 = 0.95;
/// Inflow EMA multiplier at `CONFIDENCE_NEAR`.
pub const LAMBDA_AT_NEAR: f64 = 2.0;
/// Inflow EMA multiplier at `CONFIDENCE_FAR`.
pub const LAMBDA_AT_FAR: f64 = 1.0;

/// Admit-rate floors in sat/kvB. 0.1 sat/vB steps through 10 sat/vB, then
/// 20, 50, and 100 sat/vB. The same grid is the inclusion-search ladder.
/// The last bucket is everything above the final edge.
const FLOW_BUCKET_EDGE_COUNT: usize = 103;

const fn flow_bucket_edges() -> [u64; FLOW_BUCKET_EDGE_COUNT] {
    let mut edges = [0u64; FLOW_BUCKET_EDGE_COUNT];
    let mut i = 0usize;
    let mut rate = 100u64;
    while rate <= 10_000 {
        edges[i] = rate;
        i += 1;
        rate += 100;
    }
    edges[i] = 20_000;
    i += 1;
    edges[i] = 50_000;
    i += 1;
    edges[i] = 100_000;
    edges
}

pub const FEE_BUCKET_EDGES_SAT_PER_KVB: [u64; FLOW_BUCKET_EDGE_COUNT] = flow_bucket_edges();

/// Index of the bucket that contains `rate_sat_per_kvb` (0 = lowest).
pub fn bucket_index(rate_sat_per_kvb: u64) -> usize {
    let edges = &FEE_BUCKET_EDGES_SAT_PER_KVB;
    for (i, &edge) in edges.iter().enumerate() {
        if rate_sat_per_kvb < edge {
            return i.saturating_sub(1).min(edges.len());
        }
        if rate_sat_per_kvb == edge {
            return i;
        }
    }
    // rate >= last edge → top open bucket (index == edges.len())
    // For rates in [edge[i], edge[i+1]) use i; rate >= last → edges.len()
    let mut idx = 0usize;
    for (i, &edge) in edges.iter().enumerate() {
        if rate_sat_per_kvb >= edge {
            idx = i;
        }
    }
    // Open top: rates strictly above last edge stay at last index for closed
    // buckets; treat last edge and above as last closed + one open.
    if rate_sat_per_kvb > *edges.last().unwrap_or(&0) {
        edges.len()
    } else {
        idx
    }
}

/// Number of buckets (edges + open top).
pub fn bucket_count() -> usize {
    FEE_BUCKET_EDGES_SAT_PER_KVB.len() + 1
}

/// Capacity for target block count N (≥ 1).
pub fn capacity_wu(n_blocks: u32) -> u64 {
    let n = n_blocks.max(1) as u64;
    n.saturating_mul(BLOCK_WEIGHT_WU)
}

/// Horizon seconds for target N.
pub fn horizon_secs(n_blocks: u32) -> u64 {
    let n = n_blocks.max(1) as u64;
    n.saturating_mul(SECONDS_PER_BLOCK)
}

/// Flow capacity confidence: 0.999 at N=1 and 0.99 at every farther depth.
fn inclusion_confidence(n_blocks: u32) -> f64 {
    if n_blocks <= 1 {
        CONFIDENCE_NEAR
    } else {
        CONFIDENCE_FAR
    }
}

fn lerp_conf(c: f64, y_near: f64, y_far: f64) -> f64 {
    let span = CONFIDENCE_NEAR - CONFIDENCE_FAR;
    if span <= 0.0 {
        return y_far;
    }
    let t = ((c - CONFIDENCE_FAR) / span).clamp(0.0, 1.0);
    y_far + t * (y_near - y_far)
}

/// Block-fill fraction at confidence `c` (higher `c` → less of the block).
fn fill_frac(c: f64) -> f64 {
    lerp_conf(c, FILL_AT_NEAR, FILL_AT_FAR)
}

/// Inflow EMA multiplier at confidence `c` (higher `c` → more assumed λ).
fn lambda_mult(c: f64) -> f64 {
    lerp_conf(c, LAMBDA_AT_NEAR, LAMBDA_AT_FAR)
}

/// Effective capacity after the confidence fill fraction.
pub fn effective_capacity_wu(n_blocks: u32) -> u64 {
    let fill = fill_frac(inclusion_confidence(n_blocks));
    (capacity_wu(n_blocks) as f64 * fill).round() as u64
}

/// Weight over `horizon_secs` from each bucket through the open top.
/// `suffix[i]` is buckets `i..`, and `suffix[n]` is 0.
fn inflow_suffix_wu(inflow_wu_per_s_by_bucket: &[u64], horizon_secs: u64) -> Vec<u64> {
    let n = bucket_count().min(inflow_wu_per_s_by_bucket.len());
    let mut suffix = vec![0u64; n + 1];
    for i in (0..n).rev() {
        suffix[i] =
            suffix[i + 1].saturating_add(inflow_wu_per_s_by_bucket[i].saturating_mul(horizon_secs));
    }
    suffix
}

/// First bucket whose floor is strictly above `rate`, or [`bucket_count`]
/// when every floor is at or below it.
fn first_bucket_above(rate_sat_per_kvb: u64) -> usize {
    let edges = &FEE_BUCKET_EDGES_SAT_PER_KVB;
    if let Some(i) = edges.iter().position(|&floor| floor > rate_sat_per_kvb) {
        return i;
    }
    let open_floor = edges.last().copied().unwrap_or(0).saturating_add(1);
    if open_floor > rate_sat_per_kvb {
        edges.len()
    } else {
        edges.len().saturating_add(1)
    }
}

/// Minimum feerate (sat/kvB) such that
/// `stock_above(R) + projected_in(R, H) ≤ effective_capacity(N)`.
///
/// `stock_above` is a callback (live mining-chunk weight strictly above R).
/// `candidate_rates` is searched ascending (bucket edges + optional extras).
/// Returns `None` if even the highest candidate cannot fit (caller applies floors).
pub fn min_rate_for_capacity<F>(
    stock_above: F,
    inflow_wu_per_s_by_bucket: &[u64],
    n_blocks: u32,
    candidate_rates: &[u64],
) -> Option<u64>
where
    F: Fn(u64) -> u64,
{
    let cap = effective_capacity_wu(n_blocks);
    let h = inflow_horizon_secs(n_blocks);
    let lam = lambda_mult(inclusion_confidence(n_blocks));
    let suffix = inflow_suffix_wu(inflow_wu_per_s_by_bucket, h);
    let mut best: Option<u64> = None;
    for &r in candidate_rates {
        let idx = first_bucket_above(r).min(suffix.len().saturating_sub(1));
        let stressed = (suffix[idx] as f64 * lam).round() as u64;
        let load = stock_above(r).saturating_add(stressed);
        if load <= cap {
            best = Some(match best {
                Some(b) => b.min(r),
                None => r,
            });
        }
    }
    best
}

/// Default candidate ladder: bucket edges.
pub fn default_candidate_rates() -> Vec<u64> {
    FEE_BUCKET_EDGES_SAT_PER_KVB.to_vec()
}

/// Inclusion-search ladder. Same floors as the admit-rate buckets.
pub fn fine_candidate_rates() -> Vec<u64> {
    default_candidate_rates()
}

/// Trust admit-EMA at most this many seconds (≈ 4× 150s half-life).
pub const INFLOW_HORIZON_CAP_SECS: u64 = 600;
/// Blend length in blocks (~1 hour). `w(1)=1`, `w(6)≈0.43`, `w(144)≈0`.
pub const BLEND_N0: f64 = 6.0;
/// Under-full pool may still answer min-relay when `blend_weight ≥` this (N=1–5).
pub const NEAR_BLEND_FLOOR: f64 = 0.5;

/// Horizon used for inflow projection (capped; not N×10 minutes for N=144).
pub fn inflow_horizon_secs(n_blocks: u32) -> u64 {
    horizon_secs(n_blocks).min(INFLOW_HORIZON_CAP_SECS)
}

/// Weight on the live flow estimate vs historical blocks.
pub fn blend_weight(n_blocks: u32) -> f64 {
    let n = f64::from(n_blocks.max(1));
    (-(n - 1.0) / BLEND_N0).exp()
}

/// Combine projected inflow with the N-block frontier.
///
/// If the pool is thinner than N blocks, live stock does not set a far rate
/// (`w < NEAR_BLEND_FLOOR`). Near depths with any live stock still answer
/// min-relay (everything fits now).
pub fn flow_for_depth(
    projected: Option<u64>,
    frontier: Option<u64>,
    has_live_stock: bool,
    n_blocks: u32,
    min_relay: u64,
) -> Option<u64> {
    let mut flow = match (projected, frontier) {
        (Some(p), Some(f)) => Some(p.max(f)),
        (Some(p), None) => Some(p),
        (None, Some(f)) => Some(f),
        (None, None) => None,
    };
    if frontier.is_none() {
        if blend_weight(n_blocks) >= NEAR_BLEND_FLOOR {
            if flow.is_none() && has_live_stock {
                flow = Some(min_relay);
            }
        } else {
            flow = None;
        }
    }
    flow
}

/// `pct` in 0..=100. Empty → None.
pub fn percentile_sat(mut v: Vec<u64>, pct: u8) -> Option<u64> {
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    let pct = pct.min(100) as usize;
    let i = (v.len() - 1).saturating_mul(pct) / 100;
    Some(v[i])
}

/// Vsize-weighted p10 of individual transactions in one block.
pub fn block_individual_p10_sat_kvb(txs: &[(u64, u64)], min_relay: u64) -> Option<u64> {
    use rbitcoin_consensus::policy::fee_rate_sat_per_kvb;

    let mut rates: Vec<(u64, u64)> = txs
        .iter()
        .filter_map(|&(fee, weight)| {
            if weight == 0 {
                return None;
            }
            let rate = fee_rate_sat_per_kvb(fee, weight);
            (rate >= min_relay).then_some((rate, weight.saturating_add(3) / 4))
        })
        .collect();
    if rates.is_empty() {
        return None;
    }
    rates.sort_unstable_by_key(|(rate, _)| *rate);
    let total_vsize = rates.iter().map(|(_, vsize)| *vsize).sum::<u64>();
    let cutoff = total_vsize.saturating_mul(10).div_ceil(100).max(1);
    let mut seen = 0u64;
    for (rate, vsize) in rates {
        seen = seen.saturating_add(vsize);
        if seen >= cutoff {
            return Some(rate);
        }
    }
    None
}

/// `w·flow + (1-w)·hist` with `w = blend_weight(N)`. Missing side drops out.
fn blend_sat_kvb(flow: Option<u64>, hist: Option<u64>, n_blocks: u32) -> Option<u64> {
    match (flow, hist) {
        (Some(f), Some(h)) => {
            let w = blend_weight(n_blocks);
            Some((w * f as f64 + (1.0 - w) * h as f64).round() as u64)
        }
        (flow, hist) => flow.or(hist),
    }
}

/// One target's rate (sat/kvB) before the monotone pass.
///
/// Once flow is warm, the 1-block target is flow (history only when flow has
/// nothing to say) and farther targets blend flow with history by `w(N)`, so
/// neither is a floor for the other. While flow is cold, history answers
/// alone and the live pool may only raise it: a restarted pool can be thin
/// or missing what peers relayed while this node was down.
pub fn depth_rate_sat_kvb(
    n_blocks: u32,
    flow_warm: bool,
    flow: Option<u64>,
    frontier: Option<u64>,
    hist: Option<u64>,
) -> Option<u64> {
    if !flow_warm {
        return hist.map(|h| frontier.map_or(h, |f| h.max(f)));
    }
    if n_blocks <= 1 {
        flow.or(hist)
    } else {
        blend_sat_kvb(flow, hist, n_blocks)
    }
}

/// Enforce R(1) ≥ R(2) ≥ … in place (depths already sorted ascending).
pub fn enforce_monotone_desc(rates: &mut [u64]) {
    for i in 1..rates.len() {
        if rates[i] > rates[i - 1] {
            rates[i] = rates[i - 1];
        }
    }
}

/// After the first defined depth, hold that rate through later holes, then
/// monotone. Trailing insufficient (no hist, pool thinner than N) must not
/// bounce above a cheaper mid estimate (Esplora maps -1 to 1.0 sat/vB).
pub fn hold_defined_then_monotone(rates: &mut [Option<u64>]) {
    let mut last = None;
    for r in rates.iter_mut() {
        match *r {
            Some(v) => last = Some(v),
            None => {
                if let Some(v) = last {
                    *r = Some(v);
                }
            }
        }
    }
    let mut filled: Vec<u64> = rates.iter().copied().flatten().collect();
    enforce_monotone_desc(&mut filled);
    let mut i = 0;
    for r in rates.iter_mut() {
        if r.is_some() {
            *r = Some(filled[i]);
            i += 1;
        }
    }
}

/// Confirm-target rate (sat/kvB) from already computed depths.
///
/// `depths` is ascending and the same length as `rates`. `None` is no rate.
/// An exact depth returns that depth's rate. Between two depths that both
/// have rates, the answer is linear in the block count: nearest sat/kvB,
/// halves away from zero, clamped to the two endpoints. A missing side of
/// that interval holds the nearest defined rate at or below the target. A
/// target above the last defined depth holds that rate. A target below every
/// defined depth has no rate.
pub fn fee_at_target_sat_kvb(depths: &[u32], rates: &[Option<u64>], target: u32) -> Option<u64> {
    let n = depths.len();
    if n == 0 || rates.len() != n {
        return None;
    }
    if target > depths[n - 1] {
        return rates.iter().copied().rev().find_map(|r| r);
    }
    let idx = depths.partition_point(|&d| d < target);
    if idx >= n {
        return rates.iter().copied().rev().find_map(|r| r);
    }
    if depths[idx] == target {
        return rates[idx];
    }
    if idx == 0 {
        return None;
    }
    let lo = idx - 1;
    match (rates[lo], rates[idx]) {
        (Some(left), Some(right)) => {
            Some(lerp_sat_kvb(depths[lo], left, depths[idx], right, target))
        }
        (Some(left), None) => Some(left),
        (None, _) => rates[..=lo].iter().copied().rev().find_map(|r| r),
    }
}

/// `left` at `d0`, `right` at `d1`, straight line at `target` in between.
fn lerp_sat_kvb(d0: u32, left: u64, d1: u32, right: u64, target: u32) -> u64 {
    let span = u128::from(d1.saturating_sub(d0));
    if span == 0 {
        return left;
    }
    let dist = u128::from(target.saturating_sub(d0)).min(span);
    let numer = u128::from(left) * (span - dist) + u128::from(right) * dist;
    let mut quote = numer / span;
    let rem = numer % span;
    if rem * 2 >= span {
        quote += 1;
    }
    let lo = u128::from(left.min(right));
    let hi = u128::from(left.max(right));
    quote.clamp(lo, hi) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_is_n_times_block() {
        assert_eq!(capacity_wu(1), BLOCK_WEIGHT_WU);
        assert_eq!(capacity_wu(3), 3 * BLOCK_WEIGHT_WU);
        assert_eq!(capacity_wu(0), BLOCK_WEIGHT_WU); // clamp
        assert_eq!(horizon_secs(2), 1200);
    }

    #[test]
    fn zero_inflow_equals_frontier_depth() {
        // stock_above: 2e6 WU above any rate < 1000, 0 above ≥1000
        let stock = |r: u64| if r < 1000 { 2_000_000 } else { 0 };
        let inflow = vec![0u64; bucket_count()];
        let rates = default_candidate_rates();
        let r = min_rate_for_capacity(stock, &inflow, 1, &rates).expect("fits");
        // Need stock_above(R) ≤ 0.95 * 4e6 → 2e6 always ok; minimum R on ladder that
        // still has load ≤ cap. All rates with stock 2e6 fit; min is lowest edge.
        assert_eq!(r, rates[0]);
        // Deeper N still fits at lowest.
        let r2 = min_rate_for_capacity(stock, &inflow, 5, &rates).unwrap();
        assert!(r2 <= r);
    }

    #[test]
    fn high_bucket_inflow_raises_rate() {
        let stock = |_r: u64| 0u64;
        let mut inflow = vec![0u64; bucket_count()];
        // Massive inflow in high-rate buckets (index for 50k+).
        let hi = bucket_index(50_000);
        inflow[hi] = 50_000; // WU/s → over 600s = 30e6 WU >> 4e6
        let rates = default_candidate_rates();
        let cold = min_rate_for_capacity(stock, &vec![0u64; bucket_count()], 1, &rates).unwrap();
        let hot = min_rate_for_capacity(stock, &inflow, 1, &rates).unwrap();
        assert!(hot >= cold, "hot={hot} cold={cold}");
        assert!(
            hot >= 50_000,
            "should clear high-inflow competitors, got {hot}"
        );
    }

    #[test]
    fn monotone_in_target_blocks() {
        let stock = |r: u64| if r < 5_000 { 3_500_000 } else { 0 };
        let mut inflow = vec![0u64; bucket_count()];
        inflow[bucket_index(1_000)] = 1_000; // modest
        let rates = default_candidate_rates();
        let r1 = min_rate_for_capacity(stock, &inflow, 1, &rates).unwrap();
        let r3 = min_rate_for_capacity(stock, &inflow, 3, &rates).unwrap();
        let r10 = min_rate_for_capacity(stock, &inflow, 10, &rates).unwrap();
        assert!(r3 <= r1, "r3={r3} r1={r1}");
        assert!(r10 <= r3, "r10={r10} r3={r3}");
    }

    #[test]
    fn bucket_index_edges() {
        assert_eq!(bucket_index(100), 0);
        assert_eq!(bucket_index(150), 0);
        assert_eq!(bucket_index(200), 1);
        assert!(bucket_index(1_000_000) >= FEE_BUCKET_EDGES_SAT_PER_KVB.len() - 1);
    }

    #[test]
    fn confidence_schedule_and_fill() {
        assert!((inclusion_confidence(1) - 0.999).abs() < 1e-12);
        assert!((inclusion_confidence(2) - 0.99).abs() < 1e-12);
        assert!((inclusion_confidence(1008) - 0.99).abs() < 1e-12);
        assert!((fill_frac(0.999) - 0.80).abs() < 1e-12);
        assert!((fill_frac(0.99) - 0.95).abs() < 1e-12);
        assert!((lambda_mult(0.999) - 2.0).abs() < 1e-12);
        assert!((lambda_mult(0.99) - 1.0).abs() < 1e-12);
        assert_eq!(effective_capacity_wu(1), 3_200_000);
        assert_eq!(
            effective_capacity_wu(2),
            (2.0_f64 * 4_000_000.0 * 0.95).round() as u64
        );
    }

    #[test]
    fn near_confidence_raises_rate_vs_far_fill() {
        let stock = |r: u64| if r < 5_000 { 3_500_000 } else { 0 };
        let inflow = vec![0u64; bucket_count()];
        let rates = default_candidate_rates();
        let r1 = min_rate_for_capacity(stock, &inflow, 1, &rates).unwrap();
        let r6 = min_rate_for_capacity(stock, &inflow, 6, &rates).unwrap();
        assert!(r1 >= 5_000, "N=1 80% fill cannot take 3.5e6 WU, got {r1}");
        assert_eq!(r6, rates[0], "N=6 95% of 24e6 WU fits the cheap stock");
        assert!(r1 > r6);
    }

    #[test]
    fn near_lambda_stress_raises_rate() {
        let stock = |_r: u64| 0u64;
        let mut inflow = vec![0u64; bucket_count()];
        inflow[bucket_index(1_000)] = 4_000;
        let rates = default_candidate_rates();
        let r1 = min_rate_for_capacity(stock, &inflow, 1, &rates).unwrap();
        let r6 = min_rate_for_capacity(stock, &inflow, 6, &rates).unwrap();
        assert!(r1 >= r6, "2× λ at N=1 must not undercut N=6, {r1} vs {r6}");
    }

    #[test]
    fn individual_txstat_p10_is_vsize_weighted_and_relay_filtered() {
        let rows = [(1_000, 400), (5_000, 400), (9_000, 400), (1, 400)];
        assert_eq!(block_individual_p10_sat_kvb(&rows, 100), Some(10_000));
        assert_eq!(block_individual_p10_sat_kvb(&[(1, 400)], 100), None);
        assert_eq!(block_individual_p10_sat_kvb(&[], 100), None);

        assert_eq!(
            block_individual_p10_sat_kvb(&[(1, 5), (3, 10)], 100),
            Some(500),
            "a 5-WU transaction occupies two vbytes after ceiling"
        );
        assert_eq!(
            block_individual_p10_sat_kvb(&[(1, 5), (18, 72)], 100),
            Some(500),
            "two vbytes out of twenty place the first transaction exactly at p10"
        );
    }

    #[test]
    fn blend_is_flow_at_one_and_hist_at_far() {
        assert_eq!(blend_sat_kvb(Some(5_000), Some(2_000), 1), Some(5_000));
        let r144 = blend_sat_kvb(Some(5_000), Some(2_000), 144).unwrap();
        assert!((r144 as i64 - 2_000).abs() < 5, "{r144}");
        let r2 = blend_sat_kvb(Some(5_000), Some(2_000), 2).unwrap();
        assert!(r2 > 4_000 && r2 < 5_000, "w(2) is mostly flow: {r2}");
        assert_eq!(blend_sat_kvb(Some(9_000), None, 6), Some(9_000));
        assert_eq!(blend_sat_kvb(None, Some(3_000), 6), Some(3_000));
        assert_eq!(blend_sat_kvb(None, None, 6), None);
    }

    #[test]
    fn warm_flow_drives_near_targets_without_a_history_floor() {
        // 1 block: flow alone, even far under history
        assert_eq!(
            depth_rate_sat_kvb(1, true, Some(1_000), None, Some(9_000)),
            Some(1_000)
        );
        assert_eq!(
            depth_rate_sat_kvb(1, true, None, None, Some(9_000)),
            Some(9_000)
        );
        // 2 blocks: a blend, pulled mostly toward flow
        let r2 = depth_rate_sat_kvb(2, true, Some(1_000), Some(1_000), Some(9_000)).unwrap();
        assert!(r2 > 1_000 && r2 < 3_000, "{r2}");
        let r2 = depth_rate_sat_kvb(2, true, Some(9_000), Some(9_000), Some(1_000)).unwrap();
        assert!(
            r2 > 7_000 && r2 < 9_000,
            "history does not floor flow either: {r2}"
        );
    }

    #[test]
    fn cold_flow_serves_history_that_the_pool_can_only_raise() {
        assert_eq!(
            depth_rate_sat_kvb(2, false, Some(100), Some(100), Some(4_000)),
            Some(4_000)
        );
        assert_eq!(
            depth_rate_sat_kvb(1, false, Some(100), None, Some(4_000)),
            Some(4_000)
        );
        assert_eq!(
            depth_rate_sat_kvb(2, false, None, Some(6_000), Some(4_000)),
            Some(6_000)
        );
        assert_eq!(
            depth_rate_sat_kvb(2, false, Some(6_000), Some(6_000), None),
            None
        );
    }

    #[test]
    fn hold_defined_then_monotone_fills_tail_holes() {
        let mut r = [Some(5_000), Some(900), None, None];
        hold_defined_then_monotone(&mut r);
        assert_eq!(r, [Some(5_000), Some(900), Some(900), Some(900)]);
        let mut empty = [None, None];
        hold_defined_then_monotone(&mut empty);
        assert_eq!(empty, [None, None]);
        let mut mid = [None, Some(200), None];
        hold_defined_then_monotone(&mut mid);
        assert_eq!(mid, [None, Some(200), Some(200)]);
    }

    /// Piecewise-linear curve over a fixed monotone table. Expected sat/kvB
    /// values are the arithmetic line, not a second copy of the estimator.
    #[test]
    fn fee_curve_interpolates_between_computed_depths() {
        let depths = [1, 2, 3, 4, 5, 6, 10, 20, 144, 504, 1008];
        assert_eq!(depths.len(), 11);
        let rates = [
            None,
            None,
            Some(9_000),
            Some(8_000),
            Some(7_000),
            Some(6_000),
            Some(5_000),
            Some(4_001),
            Some(3_000),
            Some(2_000),
            Some(1_000),
        ];

        assert_eq!(fee_at_target_sat_kvb(&depths, &rates, 3), Some(9_000));
        assert_eq!(fee_at_target_sat_kvb(&depths, &rates, 10), Some(5_000));
        assert_eq!(fee_at_target_sat_kvb(&depths, &rates, 1008), Some(1_000));

        let slope_depths = [10, 20];
        let slope_rates = [Some(10_000u64), Some(5_000)];
        assert_eq!(
            fee_at_target_sat_kvb(&slope_depths, &slope_rates, 10),
            Some(10_000)
        );
        assert_eq!(
            fee_at_target_sat_kvb(&slope_depths, &slope_rates, 20),
            Some(5_000)
        );
        let steps: Vec<u64> = (10..=20)
            .map(|t| fee_at_target_sat_kvb(&slope_depths, &slope_rates, t).unwrap())
            .collect();
        assert_eq!(
            steps,
            [10_000, 9_500, 9_000, 8_500, 8_000, 7_500, 7_000, 6_500, 6_000, 5_500, 5_000]
        );
        assert_ne!(steps[2], steps[0], "target 12 is not the left knot");
        let log_w = (12f64.ln() - 10f64.ln()) / (20f64.ln() - 10f64.ln());
        let log_12 = (10_000.0 + (5_000.0 - 10_000.0) * log_w).round() as u64;
        assert_ne!(steps[2], log_12, "target 12 is not a log-depth blend");

        // Halfway between 20 (4001) and 144 (3000) is block 82: 3500.5 → 3501.
        assert_eq!(fee_at_target_sat_kvb(&depths, &rates, 82), Some(3_501));

        assert_eq!(fee_at_target_sat_kvb(&depths, &rates, 1), None);
        assert_eq!(fee_at_target_sat_kvb(&depths, &rates, 2), None);
        assert_eq!(fee_at_target_sat_kvb(&slope_depths, &slope_rates, 9), None);

        assert_eq!(fee_at_target_sat_kvb(&depths, &rates, 1009), Some(1_000));
        assert_eq!(fee_at_target_sat_kvb(&depths, &rates, 5000), Some(1_000));

        let hold_only = [10u32, 20, 144];
        let hold_rates = [Some(5_000u64), None, None];
        assert_eq!(
            fee_at_target_sat_kvb(&hold_only, &hold_rates, 15),
            Some(5_000)
        );
        assert_eq!(
            fee_at_target_sat_kvb(&hold_only, &hold_rates, 100),
            Some(5_000)
        );
        assert_eq!(fee_at_target_sat_kvb(&hold_only, &hold_rates, 20), None);

        let mut prev: Option<u64> = None;
        for target in 1..=1008 {
            let rate = fee_at_target_sat_kvb(&depths, &rates, target);
            if let Some(rate) = rate {
                if let Some(prev) = prev {
                    assert!(rate <= prev, "target {target} rose above {prev} to {rate}");
                }
                prev = Some(rate);
            }
        }
        assert!(prev.is_some());
    }

    #[test]
    fn monotone_desc_clips_rises() {
        let mut r = [5_000u64, 6_000, 2_000, 3_000];
        enforce_monotone_desc(&mut r);
        assert_eq!(r, [5_000, 5_000, 2_000, 2_000]);
    }

    /// A point mass is priced at its own 0.1 sat/vB step. 1.5 must not
    /// collapse to 1.0, and 4.9 must not collapse to 2.0.
    #[test]
    fn point_mass_quotes_its_own_tenth() {
        let stock = |_r: u64| 0u64;
        let rates = fine_candidate_rates();
        // 3_000 WU/s × 600 s × λ2 = 3.6e6 WU, over the 3.2e6 N=1 cap.
        for rate in [1_500u64, 4_900] {
            let mut inflow = vec![0u64; bucket_count()];
            inflow[bucket_index(rate)] = 3_000;
            let quoted = min_rate_for_capacity(stock, &inflow, 1, &rates).unwrap();
            assert_eq!(quoted, rate, "bucket floors must be the candidate grid");
        }
    }

    #[test]
    fn inflow_horizon_is_capped() {
        assert_eq!(inflow_horizon_secs(1), 600);
        assert_eq!(inflow_horizon_secs(144), INFLOW_HORIZON_CAP_SECS);
        assert_eq!(fine_candidate_rates(), default_candidate_rates());
        assert_eq!(fine_candidate_rates()[0], 100);
        assert_eq!(*fine_candidate_rates().last().unwrap(), 100_000);
    }

    #[test]
    fn underfull_live_stock_defines_near_not_far() {
        assert!(blend_weight(5) >= NEAR_BLEND_FLOOR);
        assert!(blend_weight(6) < NEAR_BLEND_FLOOR);
        let min_r = 100u64;
        assert_eq!(flow_for_depth(None, None, true, 1, min_r), Some(min_r));
        assert_eq!(flow_for_depth(None, None, true, 5, min_r), Some(min_r));
        assert_eq!(flow_for_depth(None, None, true, 6, min_r), None);
        assert_eq!(flow_for_depth(None, None, true, 144, min_r), None);
        assert_eq!(flow_for_depth(None, None, false, 1, min_r), None);
        assert_eq!(
            flow_for_depth(Some(5_000), None, true, 1, min_r),
            Some(5_000)
        );
        assert_eq!(flow_for_depth(Some(5_000), None, true, 144, min_r), None);
        assert_eq!(
            flow_for_depth(Some(4_000), Some(3_000), true, 1, min_r),
            Some(4_000)
        );
        assert_eq!(
            flow_for_depth(None, Some(3_000), true, 144, min_r),
            Some(3_000)
        );
    }
}
