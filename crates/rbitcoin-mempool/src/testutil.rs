//! Test-only mempool hooks. Not an operator API.

use crate::store::Mempool;

/// Pretend the slot table is full so the next admit evicts instead of growing.
pub fn pin_full_slot_table(pool: &mut Mempool) {
    pool.testing_pin_full_slot_table();
}
