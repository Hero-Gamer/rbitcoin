//! Core BIP9 `testdummy` (bit 28), the one versionbits deployment Core still
//! defines. Regtest starts it at time 0 with 144-block periods and a
//! threshold of 108. Every other network has it `NEVER_ACTIVE`.

use std::collections::HashMap;
use std::sync::Mutex;

use rbitcoin_consensus::ChainParams;
use rbitcoin_primitives::Height;
use rbitcoin_store::{HeaderRecord, StoreError};

use crate::{Query, QueryError};

/// Core `VERSIONBITS_TOP_BITS`.
pub const VERSIONBITS_TOP_BITS: i32 = 0x2000_0000;
const VERSIONBITS_TOP_MASK: i32 = 0xE000_0000_u32 as i32;
pub const TESTDUMMY_BIT: u8 = 28;
const TESTDUMMY_MASK: i32 = 1 << TESTDUMMY_BIT;
const REGTEST_PERIOD: u32 = 144;
const REGTEST_THRESHOLD: usize = 108;

/// Core `ThresholdState` of a deployment for one block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThresholdState {
    Defined,
    Started,
    LockedIn,
    Active,
    Failed,
}

impl ThresholdState {
    /// Core `ComputeBlockVersion`: the bit is set while STARTED or LOCKED_IN.
    pub fn block_version(self) -> i32 {
        match self {
            Self::Started | Self::LockedIn => VERSIONBITS_TOP_BITS | TESTDUMMY_MASK,
            Self::Defined | Self::Active | Self::Failed => VERSIONBITS_TOP_BITS,
        }
    }
}

/// Core `ThresholdConditionCache`: state of the period after each period-end
/// block hash. Keyed by hash, so a reorg reads the new chain's entries.
#[derive(Default)]
pub struct VersionBitsCache(Mutex<HashMap<[u8; 32], ThresholdState>>);

impl Query {
    /// Core `GetStateFor(testdummy)` for the block after `tip`.
    pub fn testdummy_state(
        &self,
        params: &ChainParams,
        tip: Height,
    ) -> Result<ThresholdState, QueryError> {
        if params.network != bitcoin::Network::Regtest {
            return Ok(ThresholdState::Failed);
        }
        let next = tip.0.saturating_add(1);
        let period_start = next - next % REGTEST_PERIOD;
        if period_start == 0 {
            return Ok(ThresholdState::Defined);
        }
        let (_, mut end) = self
            .header_at_height(Height(period_start - 1))?
            .ok_or(StoreError::NotFound)?;
        let mut end_height = period_start - 1;
        let mut cache = self.versionbits.0.lock().expect("versionbits cache");
        let mut state = ThresholdState::Defined;
        let mut uncached = Vec::new();
        loop {
            if let Some(known) = cache.get(&end.hash) {
                state = *known;
                break;
            }
            let (signals, prev_end) = self.testdummy_period(&end, end_height)?;
            uncached.push((end.hash, signals));
            match prev_end {
                Some(prev) => {
                    end = prev;
                    end_height -= REGTEST_PERIOD;
                }
                None => break,
            }
        }
        for (hash, signals) in uncached.into_iter().rev() {
            state = match state {
                // Start time 0: every median time past is at or after it.
                ThresholdState::Defined => ThresholdState::Started,
                ThresholdState::Started if signals >= REGTEST_THRESHOLD => ThresholdState::LockedIn,
                ThresholdState::LockedIn => ThresholdState::Active,
                other => other,
            };
            cache.insert(hash, state);
        }
        Ok(state)
    }

    /// Core `VersionBitsConditionChecker::Condition` count over the period
    /// ending at `end`, read through `prev_fk` so a reorg cannot mix chains.
    /// Also returns the previous period's last header, `None` before genesis.
    fn testdummy_period(
        &self,
        end: &HeaderRecord,
        end_height: u32,
    ) -> Result<(usize, Option<HeaderRecord>), QueryError> {
        let signals = |rec: &HeaderRecord| {
            rec.version & VERSIONBITS_TOP_MASK == VERSIONBITS_TOP_BITS
                && rec.version & TESTDUMMY_MASK != 0
        };
        let mut rec = end.clone();
        let mut n = usize::from(signals(&rec));
        for _ in 1..REGTEST_PERIOD {
            rec = self.store.get_header(rec.prev_fk)?;
            n += usize::from(signals(&rec));
        }
        let prev_end = if end_height < REGTEST_PERIOD {
            None
        } else {
            Some(self.store.get_header(rec.prev_fk)?)
        };
        Ok((n, prev_end))
    }
}
