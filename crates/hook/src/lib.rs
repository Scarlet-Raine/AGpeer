//! Magnet-search backend.
//!
//! The default path is the **built-in** magnet search: generic search-engine
//! queries scoped to user-configured domains plus optional per-site search
//! templates — fully domain-neutral, zero external files. A user-configured
//! external `command` can override this with an existing scraper script.
//!
//! The discovered magnet links are normalized into the shared
//! [`agpeer_common::SearchResult`] model with the magnet URI preserved under
//! `backend_metadata["magnet"]` so a caller (or the MCP/API client) can hand
//! it to the torrent backend to actually pull the download.
//!
//! This backend is search-only: it never owns transfers. Downloads always flow
//! through the torrent backend.

mod adapter;
pub(crate) mod builtin;

pub use adapter::HookSearchBackend;
