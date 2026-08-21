//! User-configured magnet-search hook backend.
//!
//! The configured external command is run per search request and is expected to
//! emit magnet links on stdout (either one per line, or a JSON array). The
//! results are normalized into the shared [`agpeer_common::SearchResult`] model
//! with the magnet URI preserved under `backend_metadata["magnet"]` so a caller
//! (or the MCP/API client) can hand it to the torrent backend to actually pull
//! the download.
//!
//! This backend is search-only: it never owns transfers. Downloads always flow
//! through the torrent backend.

mod adapter;

pub use adapter::HookSearchBackend;
