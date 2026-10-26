//! Model-checked script-pool claim order (loom) and idle turn (shuttle).
//! Empty of those tests unless the matching `--cfg` is set.
//! Run `./scripts/model-check.sh loom` or `./scripts/model-check.sh shuttle`.
//!
//! The index-release order (`enqueue_sh_pending` then `release_index_writebehind`)
//! is not a model here. Driving it needs a `Query` full of `std` locks.

#[cfg(loom)]
mod claim;
#[cfg(any(loom, shuttle))]
mod engine;
#[cfg(shuttle)]
mod idle;

#[cfg(not(any(loom, shuttle)))]
#[test]
fn model_cfg_required() {}
