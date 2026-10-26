//! Model-checked Class C sequence counter and UTXO view stamp.
//! Empty unless `--cfg loom`. Run `./scripts/model-check.sh loom`.

#[cfg(loom)]
mod engine;
#[cfg(loom)]
mod flush;
#[cfg(loom)]
mod seq;
#[cfg(loom)]
mod utxo;

#[cfg(not(loom))]
#[test]
fn model_cfg_required() {}
