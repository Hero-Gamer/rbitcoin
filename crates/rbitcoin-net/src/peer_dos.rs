//! Per-peer DoS controls: message/byte rate windows and inbound connection caps.
//!
//! These are **not** Bitcoin Core banlist parity — they bound cheap resource abuse
//! (flooding messages or multi-MB frames) with disconnect when thresholds trip.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

/// Default max concurrent **inbound** P2P sessions (post-handshake work).
pub const DEFAULT_MAX_INBOUND: usize = 125;
/// One-second rate window (current second plus the previous second).
pub const RATE_WINDOW: Duration = Duration::from_secs(1);
/// Max application messages per peer per window (after decrypt/frame).
///
/// Tip mempool sync and compact-block reconstruction can burst many small inv /
/// getdata / tx messages; 200/s was disconnecting useful peers. 4k/s matches a
/// healthy peer under load without inviting pure message-spam (byte budget
/// still bounds bulk).
pub const DEFAULT_MAX_MSGS_PER_SEC: u32 = 4_000;
/// Max framed payload bytes per peer per window (BIP324 contents size).
/// ~16 MiB/s: enough for concurrent block + tx relay; still caps multi-peer floods.
pub const DEFAULT_MAX_BYTES_PER_SEC: u64 = 16_000_000;
/// Disconnect score added when a peer exceeds rate limits (disconnect at 100).
pub const RATE_LIMIT_BAN_SCORE: u32 = 50;
/// Disconnect score for oversized protocol messages already rejected as MessageTooLarge.
pub const OVERSIZE_BAN_SCORE: u32 = 100;

/// Process-wide inbound session slots.
pub fn inbound_semaphore(max: usize) -> Arc<Semaphore> {
    Arc::new(Semaphore::new(max.max(1)))
}

/// Per-session message and byte counters.
///
/// Two buckets (the current second and the previous one). A note weighs the
/// previous bucket by how much of it is still inside the one-second window.
/// No per-message allocation. A tumbling reset granted a second full budget
/// at the boundary.
#[derive(Debug, Clone)]
pub struct PeerRateLimiter {
    current_start: Instant,
    cur_msgs: u32,
    cur_bytes: u64,
    prev_msgs: u32,
    prev_bytes: u64,
    max_msgs: u32,
    max_bytes: u64,
}

impl PeerRateLimiter {
    pub fn new(max_msgs: u32, max_bytes: u64) -> Self {
        Self {
            current_start: Instant::now(),
            cur_msgs: 0,
            cur_bytes: 0,
            prev_msgs: 0,
            prev_bytes: 0,
            max_msgs: max_msgs.max(1),
            max_bytes: max_bytes.max(1),
        }
    }

    pub fn default_limits() -> Self {
        Self::new(DEFAULT_MAX_MSGS_PER_SEC, DEFAULT_MAX_BYTES_PER_SEC)
    }

    /// Record one framed message of `payload_len` bytes.
    /// Returns `false` if this message would exceed the window budget.
    pub fn note(&mut self, payload_len: usize) -> bool {
        self.note_at(payload_len, Instant::now())
    }

    /// Same as [`Self::note`] at a chosen instant. Tests pin the window edge.
    pub fn note_at(&mut self, payload_len: usize, now: Instant) -> bool {
        self.roll(now);
        let elapsed_ms = now
            .saturating_duration_since(self.current_start)
            .as_millis()
            .min(1_000) as u64;
        let prev_weight = 1_000 - elapsed_ms;
        let eff_msgs = u64::from(self.cur_msgs) + (u64::from(self.prev_msgs) * prev_weight) / 1_000;
        let eff_bytes = self.cur_bytes + (self.prev_bytes * prev_weight) / 1_000;
        let next_msgs = eff_msgs.saturating_add(1);
        let next_bytes = eff_bytes.saturating_add(payload_len as u64);
        if next_msgs > u64::from(self.max_msgs) || next_bytes > self.max_bytes {
            return false;
        }
        self.cur_msgs = self.cur_msgs.saturating_add(1);
        self.cur_bytes = self.cur_bytes.saturating_add(payload_len as u64);
        true
    }

    fn roll(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.current_start);
        if elapsed < RATE_WINDOW {
            return;
        }
        if elapsed >= RATE_WINDOW + RATE_WINDOW {
            self.prev_msgs = 0;
            self.prev_bytes = 0;
            self.cur_msgs = 0;
            self.cur_bytes = 0;
            self.current_start = now;
            return;
        }
        self.prev_msgs = self.cur_msgs;
        self.prev_bytes = self.cur_bytes;
        self.cur_msgs = 0;
        self.cur_bytes = 0;
        self.current_start += RATE_WINDOW;
    }
}

/// Count one decoy (or other unsolicited frame) in `rate`.
///
/// A frame that does not fit adds [`RATE_LIMIT_BAN_SCORE`] and stays
/// connected until `disconnect_at`. Callers use the same threshold as an
/// unknown message type.
pub fn decoy_stays(
    rate: &mut PeerRateLimiter,
    ban_score: &mut u32,
    payload_len: usize,
    disconnect_at: u32,
) -> bool {
    if rate.note(payload_len) {
        return true;
    }
    *ban_score = ban_score.saturating_add(RATE_LIMIT_BAN_SCORE);
    *ban_score < disconnect_at
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limiter_allows_under_budget() {
        let mut r = PeerRateLimiter::new(10, 1000);
        for _ in 0..10 {
            assert!(r.note(50));
        }
        assert!(!r.note(1), "11th message must trip msg limit");
    }

    #[test]
    fn rate_limiter_bytes_cap() {
        let mut r = PeerRateLimiter::new(1000, 100);
        assert!(r.note(100));
        assert!(!r.note(1));

        let mut blended = PeerRateLimiter::new(10_000, 2_000);
        let t0 = Instant::now();
        assert!(blended.note_at(1_000, t0));
        assert!(blended.note_at(1, t0 + RATE_WINDOW));
        assert!(
            !blended.note_at(1_600, t0 + RATE_WINDOW + Duration::from_millis(500)),
            "500ms later half of the previous 1000 bytes plus this frame exceeds 2000"
        );
    }

    #[test]
    fn rate_limiter_boundary_does_not_grant_a_second_budget() {
        let mut r = PeerRateLimiter::new(2, 10_000);
        let t0 = Instant::now();
        assert!(r.note_at(1, t0));
        assert!(r.note_at(1, t0));
        assert!(!r.note_at(1, t0));
        let boundary = t0 + RATE_WINDOW;
        assert!(
            !r.note_at(1, boundary),
            "a full previous second must not grant another budget at the boundary"
        );
        let cleared = t0 + RATE_WINDOW + RATE_WINDOW + Duration::from_millis(1);
        assert!(r.note_at(1, cleared));
        assert!(r.note_at(1, cleared));
        assert!(!r.note_at(1, cleared));

        let mut rolled = PeerRateLimiter::new(10_000, 1_000);
        let roll_t0 = Instant::now();
        assert!(rolled.note_at(800, roll_t0));
        assert!(rolled.note_at(100, roll_t0 + RATE_WINDOW));
        assert!(
            !rolled.note_at(510, roll_t0 + RATE_WINDOW + Duration::from_millis(500)),
            "rolled: 100 + half of 800 + 510 exceeds 1000"
        );

        // `new` stamps its own start. Jump far enough that this note sets
        // the window start to `anchor`, then hit that start plus one second.
        let mut edge = PeerRateLimiter::new(10_000, 1_000);
        let anchor = Instant::now() + RATE_WINDOW + RATE_WINDOW + Duration::from_secs(5);
        assert!(edge.note_at(800, anchor));
        let boundary = anchor + RATE_WINDOW;
        assert!(
            edge.note_at(200, boundary),
            "the exact boundary still has the rest of a 1000-byte budget"
        );
        assert!(
            !edge.note_at(401, boundary + Duration::from_millis(500)),
            "half a second later the previous 800 bytes still count"
        );
    }

    #[test]
    fn rate_limiter_window_resets() {
        let mut r = PeerRateLimiter::new(2, 10_000);
        let t0 = Instant::now();
        assert!(r.note_at(1, t0));
        assert!(r.note_at(1, t0));
        assert!(!r.note_at(1, t0));
        let cleared = t0 + RATE_WINDOW + RATE_WINDOW + Duration::from_millis(1);
        assert!(r.note_at(1, cleared));
    }

    #[test]
    fn max_inbound_env_default() {
        // Do not mutate env in parallel tests; just check parse of default path.
        const {
            assert!(DEFAULT_MAX_INBOUND >= 1);
        }
        assert_eq!(RATE_LIMIT_BAN_SCORE, 50);
        assert_eq!(OVERSIZE_BAN_SCORE, 100);
    }

    #[test]
    fn one_overflow_scores_and_the_second_disconnects() {
        let mut rate = PeerRateLimiter::new(1, 10_000);
        let mut score = 0u32;
        let disconnect_at = RATE_LIMIT_BAN_SCORE.saturating_mul(2);
        assert!(decoy_stays(&mut rate, &mut score, 1, disconnect_at));
        assert_eq!(score, 0, "a frame that fits does not score");
        assert!(
            decoy_stays(&mut rate, &mut score, 1, disconnect_at),
            "one frame over the window stays connected"
        );
        assert_eq!(score, RATE_LIMIT_BAN_SCORE);
        assert!(
            !decoy_stays(&mut rate, &mut score, 1, disconnect_at),
            "the next overflow reaches the disconnect threshold"
        );
        assert_eq!(score, disconnect_at);
    }
}
