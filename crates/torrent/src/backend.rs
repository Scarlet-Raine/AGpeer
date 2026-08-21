//! The public torrent backend façade implementing
//! [`agpeer_common::TransferBackend`].
//!
//! [`TorrentBackend::new`] wires the in-memory reference engine
//! (`memory::MemoryEngine`); the real librqbit engine is available behind the
//! `rqbit` cargo feature via [`TorrentBackend::new_rqbit`].

use agpeer_common::{
    AddTransferRequest, Backend, Result as CommonResult, Transfer, TransferBackend, TransferId,
};
use async_trait::async_trait;

use crate::config::TorrentConfig;
use crate::engine::TorrentEngine;
use crate::error::BackendError;
use crate::memory::MemoryEngine;

#[cfg(feature = "rqbit")]
use crate::rqbit::RqbitEngine;

/// A torrent `TransferBackend` over an internal engine.
pub struct TorrentBackend {
    engine: Box<dyn TorrentEngine>,
}

impl TorrentBackend {
    /// Create a backend over the in-memory reference engine.
    ///
    /// The reference engine is fully functional (validate/add/list/get/pause/
    /// resume/cancel/shutdown) but does not perform real BitTorrent transfers;
    /// it is intended for development, tests, and as the documented default
    /// until the librqbit engine is verified.
    pub async fn new(config: TorrentConfig) -> Result<Self, BackendError> {
        Ok(Self {
            engine: Box::new(MemoryEngine::new(config)),
        })
    }

    /// Create a backend over the real librqbit engine.
    ///
    /// Requires the `rqbit` cargo feature. See `SPIKE.md` for status.
    #[cfg(feature = "rqbit")]
    pub async fn new_rqbit(config: TorrentConfig) -> Result<Self, BackendError> {
        Ok(Self {
            engine: Box::new(RqbitEngine::new(config).await?),
        })
    }

    /// The name of the underlying engine (`"memory"` or `"rqbit"`).
    pub fn engine_name(&self) -> &'static str {
        self.engine.engine_name()
    }

    /// Shut the backend down, releasing engine resources.
    pub async fn shutdown(self) -> Result<(), BackendError> {
        self.engine.shutdown().await
    }
}

#[async_trait]
impl TransferBackend for TorrentBackend {
    fn backend(&self) -> Backend {
        Backend::Torrent
    }

    async fn add(&self, request: AddTransferRequest) -> CommonResult<Transfer> {
        Ok(self.engine.add(request).await?)
    }

    async fn get(&self, id: &TransferId) -> CommonResult<Transfer> {
        Ok(self.engine.get(id).await?)
    }

    async fn list(&self) -> CommonResult<Vec<Transfer>> {
        Ok(self.engine.list().await?)
    }

    async fn pause(&self, id: &TransferId) -> CommonResult<()> {
        self.engine.pause(id).await?;
        Ok(())
    }

    async fn resume(&self, id: &TransferId) -> CommonResult<()> {
        self.engine.resume(id).await?;
        Ok(())
    }

    async fn cancel(&self, id: &TransferId, delete_data: bool) -> CommonResult<()> {
        self.engine.cancel(id, delete_data).await?;
        Ok(())
    }

    async fn forget(&self, id: &TransferId) -> CommonResult<()> {
        self.engine.forget(id).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[tokio::test]
    async fn new_wires_the_memory_engine_and_implements_transfer_backend() {
        let dir = tempfile::tempdir().unwrap();
        let config = TorrentConfig {
            download_root: dir.path().to_string_lossy().into_owned(),
            ..TorrentConfig::default()
        };
        let backend = TorrentBackend::new(config).await.unwrap();
        assert_eq!(backend.engine_name(), "memory");
        assert_eq!(backend.backend(), Backend::Torrent);

        let request = AddTransferRequest {
            backend: Backend::Torrent,
            source: "magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862".into(),
            destination: None,
            display_name: None,
            file_selection: None,
            metadata: HashMap::new(),
        };
        let transfer = backend.add(request).await.unwrap();
        assert_eq!(transfer.id, backend.get(&transfer.id).await.unwrap().id);
        assert_eq!(backend.list().await.unwrap().len(), 1);

        backend.shutdown().await.unwrap();
    }
}
