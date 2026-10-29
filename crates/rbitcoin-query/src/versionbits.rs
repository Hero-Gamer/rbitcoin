//! Core BIP9 `testdummy` (bit 28), the one versionbits deployment Core still
//! defines. Regtest starts it at time 0 with 144-block periods and a
//! threshold of 108. Every other network has it `NEVER_ACTIVE`.

use std::collections::HashMap;
use std::sync::Mutex;

use rbitcoin_consensus::ChainParams;
use rbitcoin_primitives::Height;
use rbitcoin_store::StoreError;

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
        let mut cache = self.versionbits.0.lock().expect("versionbits cache");
        let mut state = ThresholdState::Defined;
        let mut uncached = Vec::new();
        let mut period_start = next - next % REGTEST_PERIOD;
        while period_start >= REGTEST_PERIOD {
            let end = Height(period_start - 1);
            let hash = self.confirmed_hash_at(end)?;
            if let Some(known) = cache.get(&hash) {
                state = *known;
                break;
            }
            uncached.push((end, hash));
            period_start -= REGTEST_PERIOD;
        }
        for (end, hash) in uncached.into_iter().rev() {
            state = match state {
                // Start time 0: every median time past is at or after it.
                ThresholdState::Defined => ThresholdState::Started,
                ThresholdState::Started if self.testdummy_signals(end)? >= REGTEST_THRESHOLD => {
                    ThresholdState::LockedIn
                }
                ThresholdState::LockedIn => ThresholdState::Active,
                other => other,
            };
            cache.insert(hash, state);
        }
        Ok(state)
    }

    fn confirmed_hash_at(&self, height: Height) -> Result<[u8; 32], QueryError> {
        Ok(self
            .header_at_height(height)?
            .ok_or(StoreError::NotFound)?
            .1
            .hash)
    }

    /// Core `VersionBitsConditionChecker::Condition` over the period ending at `end`.
    fn testdummy_signals(&self, end: Height) -> Result<usize, QueryError> {
        let mut n = 0;
        for h in end.0 + 1 - REGTEST_PERIOD..=end.0 {
            let (_, rec) = self
                .header_at_height(Height(h))?
                .ok_or(StoreError::NotFound)?;
            if rec.version & VERSIONBITS_TOP_MASK == VERSIONBITS_TOP_BITS
                && rec.version & TESTDUMMY_MASK != 0
            {
                n += 1;
            }
        }
        Ok(n)
    }
}
