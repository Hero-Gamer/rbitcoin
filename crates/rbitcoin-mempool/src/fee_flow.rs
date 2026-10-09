//! Process-local fee flow meter (EMA of admit WU/s per bucket).

use crate::fee_est::{bucket_count, bucket_index, BLOCK_WEIGHT_WU, SECONDS_PER_BLOCK};
use std::time::Instant;

/// Admit EMA half-life (seconds). The fullness scalar uses the same half-life.
pub const ADMIT_HALF_LIFE_SECS: f64 = 150.0;

/// Weight a full block every 10 minutes leaves in the half-life memory.
pub fn flow_full_wu() -> f64 {
    (BLOCK_WEIGHT_WU as f64 / SECONDS_PER_BLOCK as f64)
        * (ADMIT_HALF_LIFE_SECS / std::f64::consts::LN_2)
}

/// EMA flow state for fee Engine v2.
#[derive(Debug, Clone)]
pub struct FeeFlowMeter {
    admit_wu_s: Vec<f64>,
    /// Decayed admitted weight (WU). Not the per-bucket time-gap alpha.
    admitted_wu: f64,
    last: Instant,
}

impl Default for FeeFlowMeter {
    fn default() -> Self {
        Self::new(Instant::now())
    }
}

impl FeeFlowMeter {
    pub fn new(now: Instant) -> Self {
        let n = bucket_count();
        Self {
            admit_wu_s: vec![0.0; n],
            admitted_wu: 0.0,
            last: now,
        }
    }

    /// Fullness in `0..=1` after decaying to `now`. Starts at 0.
    pub fn fullness(&mut self, now: Instant) -> f64 {
        self.decay_to(now);
        let full = flow_full_wu();
        if full <= 0.0 {
            return 0.0;
        }
        (self.admitted_wu / full).clamp(0.0, 1.0)
    }

    /// Snapshot of admit λ (WU/s) per bucket after decaying to `now`.
    pub fn admit_rates_wu_s(&mut self, now: Instant) -> Vec<u64> {
        self.decay_to(now);
        self.admit_wu_s
            .iter()
            .map(|x| x.max(0.0).round() as u64)
            .collect()
    }

    pub fn note_admit(&mut self, weight_wu: u64, rate_sat_per_kvb: u64, now: Instant) {
        let dt = now.duration_since(self.last).as_secs_f64().max(1e-3);
        self.decay_to(now);
        let i = bucket_index(rate_sat_per_kvb).min(self.admit_wu_s.len().saturating_sub(1));
        let sample = weight_wu as f64 / dt;
        let alpha = 1.0 - (-std::f64::consts::LN_2 * dt / ADMIT_HALF_LIFE_SECS).exp();
        self.admit_wu_s[i] = self.admit_wu_s[i] * (1.0 - alpha) + sample * alpha;
        self.admitted_wu += weight_wu as f64;
    }

    fn decay_to(&mut self, now: Instant) {
        let dt = now.duration_since(self.last).as_secs_f64();
        if dt <= 0.0 {
            return;
        }
        let factor_a = (-std::f64::consts::LN_2 * dt / ADMIT_HALF_LIFE_SECS).exp();
        for v in &mut self.admit_wu_s {
            *v *= factor_a;
        }
        self.admitted_wu *= factor_a;
        self.last = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fee_est::{BLOCK_WEIGHT_WU, SECONDS_PER_BLOCK};
    use std::time::Duration;

    #[test]
    fn admit_stream_raises_ema() {
        let t0 = Instant::now();
        let mut m = FeeFlowMeter::new(t0);
        for i in 0..40 {
            let t = t0 + Duration::from_secs(i + 1);
            m.note_admit(10_000, 1_000, t);
        }
        let rates = m.admit_rates_wu_s(t0 + Duration::from_secs(41));
        let bi = bucket_index(1_000);
        assert!(rates[bi] > 0, "ema should be warm, got {rates:?}");
    }

    #[test]
    fn fullness_tracks_decayed_weight_not_the_clock() {
        let t0 = Instant::now();
        let mut idle = FeeFlowMeter::new(t0);
        assert_eq!(idle.fullness(t0 + Duration::from_secs(10_000)), 0.0);

        let mut one = FeeFlowMeter::new(t0);
        one.note_admit(1_000, 1_000, t0 + Duration::from_secs(1_000));
        let after_gap = one.fullness(t0 + Duration::from_secs(1_000));
        assert!(
            after_gap < 0.01,
            "one tx after a long pause must not fill the meter, got {after_gap}"
        );

        let mut small = FeeFlowMeter::new(t0);
        for i in 0..32 {
            small.note_admit(1_000, 1_000, t0 + Duration::from_secs(i + 1));
        }
        let young = small.fullness(t0 + Duration::from_secs(33));
        assert!(
            young < 0.05,
            "32 ordinary admits stay near zero, got {young}"
        );

        let full = flow_full_wu().round() as u64;
        let mut lump = FeeFlowMeter::new(t0);
        lump.note_admit(full, 1_000, t0 + Duration::from_secs(1));
        let hot = lump.fullness(t0 + Duration::from_secs(1));
        assert!(hot > 0.99, "a full block of weight fills α, got {hot}");
        let cooled = lump.fullness(t0 + Duration::from_secs(1 + 150));
        assert!(
            (cooled - 0.5).abs() < 0.02,
            "half-life decay of the scalar, got {cooled}"
        );
        let expect = (BLOCK_WEIGHT_WU as f64 / SECONDS_PER_BLOCK as f64)
            * (ADMIT_HALF_LIFE_SECS / std::f64::consts::LN_2);
        assert!((flow_full_wu() - expect).abs() < 1.0, "{}", flow_full_wu());
    }

    #[test]
    fn idle_decays() {
        let t0 = Instant::now();
        let mut m = FeeFlowMeter::new(t0);
        m.note_admit(100_000, 500, t0 + Duration::from_secs(1));
        let hot = m.admit_rates_wu_s(t0 + Duration::from_secs(2));
        let cold = m.admit_rates_wu_s(t0 + Duration::from_secs(600));
        let bi = bucket_index(500);
        assert!(cold[bi] < hot[bi], "hot={} cold={}", hot[bi], cold[bi]);
    }
}
