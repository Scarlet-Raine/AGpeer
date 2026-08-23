//! Shared typed models, opaque identifiers, and error types.
//!
//! This crate is the single source of truth for the normalized transfer and
//! search model that every backend and every client (API, UI, MCP) consumes.

mod backend;
mod error;
mod hook;
mod ids;
mod search;
mod transfer;
mod util;

pub use backend::{SearchBackend, TransferBackend};
pub use error::{ApiError, Error, Result};
pub use hook::{ExtractStrategy, HookSearchSite};
pub use ids::{ResultId, SearchId, TransferId};
pub use search::{Search, SearchRequest, SearchResult, SearchState};
pub use transfer::{
    AddTransferRequest, AddTransferResponse, Backend, DownloadResultRequest, FileSelection,
    PostprocessState, Transfer, TransferFile, TransferState,
};
pub use util::percent_decode;
