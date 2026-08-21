//! Backend abstraction traits.
//!
//! Every transfer/search backend normalizes into these traits so the rest of
//! the application never touches provider-specific types.

use crate::{
    AddTransferRequest, Result, SearchId, SearchRequest, SearchResult, Transfer, TransferId,
};
use async_trait::async_trait;

/// A backend that owns transfers.
#[async_trait]
pub trait TransferBackend: Send + Sync {
    /// Backend identifier used in the normalized model.
    fn backend(&self) -> crate::Backend;

    /// Add a new transfer and return its normalized id.
    async fn add(&self, request: AddTransferRequest) -> Result<Transfer>;

    /// Fetch a single transfer by id.
    async fn get(&self, id: &TransferId) -> Result<Transfer>;

    /// List all transfers known to this backend.
    async fn list(&self) -> Result<Vec<Transfer>>;

    /// Pause a transfer.
    async fn pause(&self, id: &TransferId) -> Result<()>;

    /// Resume a paused transfer.
    async fn resume(&self, id: &TransferId) -> Result<()>;

    /// Cancel a transfer, optionally deleting downloaded data.
    async fn cancel(&self, id: &TransferId, delete_data: bool) -> Result<()>;

    /// Remove the transfer record from the backend without touching downloaded
    /// data. Used when reconciling transfers that are missing from the backend.
    async fn forget(&self, id: &TransferId) -> Result<()>;
}

/// A backend that can run searches (Soulseek only in v1).
#[async_trait]
pub trait SearchBackend: Send + Sync {
    /// Backend identifier used in the normalized model.
    fn backend(&self) -> crate::Backend;

    /// Start a search and return its normalized id.
    async fn search(&self, request: SearchRequest) -> Result<SearchId>;

    /// Fetch the currently-accumulated results for a search.
    async fn results(&self, id: &SearchId) -> Result<Vec<SearchResult>>;

    /// Stop/expire a search.
    async fn stop(&self, id: &SearchId) -> Result<()>;
}
