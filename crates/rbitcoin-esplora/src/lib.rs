//! Esplora-compatible REST HTTP for **wallet clients and APIs** (plain HTTP;
//! TLS via reverse proxy).
//!
//! Serves exact address/scripthash history, tx/block by id, and broadcast—not a
//! graphical block-explorer product (no address-prefix search / explorer UI
//! catalogue APIs). Opt-in `GET /block-template` is GBT, not explorer search.

mod handlers;
mod script_fields;
mod server;
mod tx_json;
mod ws;

pub use server::{run_esplora, sample_reset_perf, BlockTemplateFn, EsploraConfig, EsploraHandle};
