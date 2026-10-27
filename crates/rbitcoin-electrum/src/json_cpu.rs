//! JSON replies. Under 4 KiB encodes on the caller. At 4 KiB and above,
//! encoding runs on `rbtc-scripts-*`. The store join stays where it was.

use serde_json::Value;
use std::sync::Arc;

/// Encoded bodies at or above this leave the caller for `rbtc-scripts-*`.
pub const JSON_CPU_MIN: usize = 4096;

/// True when a lower bound of the encoded size reaches [`JSON_CPU_MIN`].
/// The walk stops at that bound.
#[doc(hidden)]
pub fn json_is_cpu_heavy(v: &Value) -> bool {
    let mut left = JSON_CPU_MIN;
    walk(v, &mut left)
}

fn take(left: &mut usize, n: usize) -> bool {
    if n >= *left {
        *left = 0;
        true
    } else {
        *left -= n;
        false
    }
}

fn walk(v: &Value, left: &mut usize) -> bool {
    if *left == 0 {
        return true;
    }
    match v {
        Value::Null => take(left, 4),
        Value::Bool(true) => take(left, 4),
        Value::Bool(false) => take(left, 5),
        Value::Number(_) => take(left, 1),
        Value::String(s) => take(left, s.len().saturating_add(2)),
        Value::Array(items) => {
            if take(left, 1) {
                return true;
            }
            for (i, item) in items.iter().enumerate() {
                if i > 0 && take(left, 1) {
                    return true;
                }
                if walk(item, left) {
                    return true;
                }
            }
            take(left, 1)
        }
        Value::Object(map) => {
            if take(left, 1) {
                return true;
            }
            for (i, (key, item)) in map.iter().enumerate() {
                if i > 0 && take(left, 1) {
                    return true;
                }
                if take(left, key.len().saturating_add(3)) || walk(item, left) {
                    return true;
                }
            }
            take(left, 1)
        }
    }
}

/// Encode `v`. A heavy body hops to `rbtc-scripts-*`. A light body encodes here.
///
/// The `Arc` moves into the script job. Callers that already own the value
/// do not clone it on the waiting task. `Err` means the worker dropped the
/// reply.
#[doc(hidden)]
pub async fn encode_json_body(v: Arc<Value>) -> Result<Vec<u8>, ()> {
    if !json_is_cpu_heavy(&v) {
        return Ok(serde_json::to_vec(v.as_ref()).unwrap_or_else(|_| b"{}".to_vec()));
    }
    rbitcoin_consensus::script_pool::run_cpu(move || {
        serde_json::to_vec(v.as_ref()).unwrap_or_else(|_| b"{}".to_vec())
    })
    .await
    .ok_or(())
}
