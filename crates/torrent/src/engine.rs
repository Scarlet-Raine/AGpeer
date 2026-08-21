//! The internal engine abstraction every torrent backend implementation
//! satisfies.
//!
//! This is the seam that lets the real librqbit engine and the in-memory
//! reference engine share one normalized API without the rest of the
//! application knowing which one is wired.

use agpeer_common::{AddTransferRequest, Transfer, TransferId};
use async_trait::async_trait;

use crate::error::BackendError;

/// Operations every torrent engine must support.
///
/// Engines own their state and return normalized `Transfer` snapshots.
#[async_trait]
pub(crate) trait TorrentEngine: Send + Sync {
    /// Short engine identifier (`"memory"` / `"rqbit"`) for diagnostics.
    fn engine_name(&self) -> &'static str;

    /// Validate and add a transfer, returning its normalized snapshot.
    async fn add(&self, request: AddTransferRequest) -> Result<Transfer, BackendError>;

    /// Fetch a single transfer snapshot.
    async fn get(&self, id: &TransferId) -> Result<Transfer, BackendError>;

    /// List all transfers known to the engine.
    async fn list(&self) -> Result<Vec<Transfer>, BackendError>;

    /// Pause an active transfer.
    async fn pause(&self, id: &TransferId) -> Result<(), BackendError>;

    /// Resume a paused transfer.
    async fn resume(&self, id: &TransferId) -> Result<(), BackendError>;

    /// Cancel a transfer, optionally deleting downloaded data.
    async fn cancel(&self, id: &TransferId, delete_data: bool) -> Result<(), BackendError>;

    /// Remove the transfer record from the backend without touching downloaded
    /// data. Used when reconciling transfers that are missing from the backend.
    async fn forget(&self, id: &TransferId) -> Result<(), BackendError>;

    /// Shut the engine down, releasing any resources.
    async fn shutdown(self: Box<Self>) -> Result<(), BackendError>;
}
