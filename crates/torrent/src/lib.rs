//! BitTorrent transfer backend for agpeer.
//!
//! Implements [`agpeer_common::TransferBackend`] over an internal
//! `TorrentEngine` abstraction.
//!
//! Two engines exist behind that trait:
//!
//! - `memory::MemoryEngine` — an in-memory reference engine. It validates
//!   sources, parses `.torrent` metainfo (via a minimal bencode reader), tracks
//!   per-transfer state and simulates progress advancing over time. It is the
//!   DEFAULT engine wired by `backend::TorrentBackend::new` and is fully
//!   unit-tested.
//! - `rqbit::RqbitEngine` — a real librqbit-backed engine. Compiled only behind
//!   the `rqbit` cargo feature (off by default). See `SPIKE.md` for the status
//!   of this integration.
//!
//! All backend-specific metadata is namespaced under the `"torrent"` key of
//! `Transfer::metadata`.

mod bencode;
mod engine;
mod memory;
mod normalize;
mod source;

#[cfg(feature = "rqbit")]
mod rqbit;

pub mod backend;
pub mod config;
pub mod error;

pub use backend::TorrentBackend;
pub use config::TorrentConfig;
pub use error::BackendError;
