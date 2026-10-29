//! Core BIP9 `testdummy`, the one versionbits deployment Core still defines.
//! Regtest starts it at time 0 ([`ChainParams::testdummy`]). Every other
//! network has it `NEVER_ACTIVE` (`None`).

use std::collections::HashMap;
use std::sync::Mutex;

use rbitcoin_consensus::{Bip9Deployment, ChainParams};
use rbitcoin_primitives::Height;
use rbitcoin_store::{HeaderRecord, StoreError};

use crate::{Query, QueryError};

/// Core `VERSIONBITS_TOP_BITS`.
pub const VERSIONBITS_TOP_BITS: i32 = 0x2000_0000;
const VERSIONBITS_TOP_MASK: i32 = 0xE000_0000_u32 as i32;

/// Core `ThresholdState` of a deployment that cannot time out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThresholdState {
    Defined,
    Started,
    LockedIn,
    Active,
}

/// A deployment's bit and its state for one block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeploymentState {
    pub bit: u8,
    pub state: ThresholdState,
}

/// Core `ComputeBlockVersion`: the bit is set while STARTED or LOCKED_IN.
pub fn compute_block_version(testdummy: Option<DeploymentState>) -> i32 {
    match testdummy {
        Some(d) if matches!(d.state, ThresholdState::Started | ThresholdState::LockedIn) => {
            VERSIONBITS_TOP_BITS | (1 << d.bit)
        }
        _ => VERSIONBITS_TOP_BITS,
    }
}

/// Core `ThresholdConditionCache`: state of the period after each period-end
/// block hash. Keyed by hash, so a reorg reads the new chain's entries. Only a
/// network with a deployment inserts, one entry per period-end evaluated.
#[derive(Default)]
pub struct VersionBitsCache(Mutex<HashMap<[u8; 32], ThresholdState>>);

impl VersionBitsCache {
    fn get(&self, hash: &[u8; 32]) -> Option<ThresholdState> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(hash)
            .copied()
    }

    fn insert(&self, hash: [u8; 32], state: ThresholdState) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(hash, state);
    }
}

impl Query {
    /// Core `GetStateFor(testdummy)` for the block after `tip`.
    pub fn testdummy_state(
        &self,
        params: &ChainParams,
        tip: Height,
    ) -> Result<Option<DeploymentState>, QueryError> {
        let Some(dep) = params.testdummy else {
            return Ok(None);
        };
        let state = self.deployment_state(&dep, tip)?;
        Ok(Some(DeploymentState {
            bit: dep.bit,
            state,
        }))
    }

    fn deployment_state(
        &self,
        dep: &Bip9Deployment,
        tip: Height,
    ) -> Result<ThresholdState, QueryError> {
        let next = tip.0.saturating_add(1);
        let period_start = next - next % dep.period;
        if period_start == 0 {
            return Ok(ThresholdState::Defined);
        }
        let (_, mut end) = self
            .header_at_height(Height(period_start - 1))?
            .ok_or(StoreError::NotFound)?;
        let mut end_height = period_start - 1;
        let mut state = ThresholdState::Defined;
        let mut uncached = Vec::new();
        loop {
            if let Some(known) = self.versionbits.get(&end.hash) {
                state = known;
                break;
            }
            let (signals, prev_end) = self.period_signals(dep, &end, end_height)?;
            uncached.push((end.hash, signals));
            match prev_end {
                Some(prev) => {
                    end = prev;
                    end_height -= dep.period;
                }
                None => break,
            }
        }
        for (hash, signals) in uncached.into_iter().rev() {
            state = match state {
                // Start time 0: every median time past is at or after it.
                ThresholdState::Defined => ThresholdState::Started,
                ThresholdState::Started if signals >= dep.threshold => ThresholdState::LockedIn,
                ThresholdState::LockedIn => ThresholdState::Active,
                other => other,
            };
            self.versionbits.insert(hash, state);
        }
        Ok(state)
    }

    /// Core `VersionBitsConditionChecker::Condition` count over the period
    /// ending at `end`, read through `prev_fk` so a reorg cannot mix chains.
    /// Also returns the previous period's last header, `None` before genesis.
    fn period_signals(
        &self,
        dep: &Bip9Deployment,
        end: &HeaderRecord,
        end_height: u32,
    ) -> Result<(u32, Option<HeaderRecord>), QueryError> {
        let mask = 1 << dep.bit;
        let signals = |rec: &HeaderRecord| {
            rec.version & VERSIONBITS_TOP_MASK == VERSIONBITS_TOP_BITS && rec.version & mask != 0
        };
        let mut rec = end.clone();
        let mut n = u32::from(signals(&rec));
        for _ in 1..dep.period {
            rec = self.store.get_header(rec.prev_fk)?;
            n += u32::from(signals(&rec));
        }
        let prev_end = if end_height < dep.period {
            None
        } else {
            Some(self.store.get_header(rec.prev_fk)?)
        };
        Ok((n, prev_end))
    }
}
