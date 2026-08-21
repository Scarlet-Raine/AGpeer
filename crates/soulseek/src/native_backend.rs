//! Soulseek backend adapter over the native [`rustsoseek`] client.
//!
//! Maps the client's self-contained search/transfer surface into the shared
//! [`agpeer_common::SearchBackend`] and [`agpeer_common::TransferBackend`]
//! traits, including the application-owned opaque IDs and backend metadata.
//!
//! Search results are held in memory (capped); transfer state is tracked in
//! memory and reconciled from the client's download progress.

use agpeer_common::{
    AddTransferRequest, Backend, SearchId, SearchRequest, SearchResult, Transfer, TransferId,
    TransferState,
};
use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rustsoseek::{NativeClient, NativeConfig};

use crate::error::map_error;

/// In-memory state owned by the backend.
#[derive(Default)]
struct BackendState {
    // app SearchId -> native search token
    searches: HashMap<String, u32>,
    // opaque ResultId -> (username, filename, size)
    results: HashMap<String, StoredResult>,
    // app TransferId -> Transfer
    transfers: HashMap<String, Transfer>,
}

#[derive(Clone)]
struct StoredResult {
    username: String,
    filename: String,
    size: Option<u64>,
}

/// Native Soulseek backend implementing both search and transfer.
pub struct NativeSoulseekBackend {
    config: NativeConfig,
    client: NativeClient,
    state: Arc<Mutex<BackendState>>,
}

impl NativeSoulseekBackend {
    /// Connect the native client and build the backend. Returns an error if
    /// the Soulseek server cannot be reached or login is rejected.
    pub async fn connect(config: NativeConfig) -> Result<Self, agpeer_common::Error> {
        let client = NativeClient::connect(config.clone())
            .await
            .map_err(map_error)?;
        Ok(Self {
            config,
            client,
            state: Arc::new(Mutex::new(BackendState::default())),
        })
    }

    /// The client's listen address (peers connect here for search responses).
    pub fn listen_addr(&self) -> std::net::SocketAddr {
        self.client.listen_addr()
    }
}

#[async_trait]
impl agpeer_common::SearchBackend for NativeSoulseekBackend {
    fn backend(&self) -> Backend {
        Backend::Soulseek
    }

    async fn search(&self, request: SearchRequest) -> Result<SearchId, agpeer_common::Error> {
        if request.backend != Backend::Soulseek {
            return Err(agpeer_common::Error::InvalidSource);
        }
        let app_id = SearchId::new();
        let token = self
            .client
            .start_search(&request.query)
            .await
            .map_err(map_error)?;
        self.state
            .lock()
            .unwrap()
            .searches
            .insert(app_id.to_string(), token);
        Ok(app_id)
    }

    async fn results(&self, id: &SearchId) -> Result<Vec<SearchResult>, agpeer_common::Error> {
        let token = {
            let state = self.state.lock().unwrap();
            state
                .searches
                .get(&id.to_string())
                .copied()
                .ok_or(agpeer_common::Error::SearchNotFound)?
        };

        let native = self.client.results(token, 1000);
        let results: Vec<SearchResult> = native.into_iter().map(|r| map_result(r, *id)).collect();

        // Cache the results so a later download request can resolve the
        // username/filename/size from the opaque result id.
        {
            let mut state = self.state.lock().unwrap();
            for r in &results {
                state.results.insert(
                    r.result_id.to_string(),
                    StoredResult {
                        username: r.username.clone(),
                        filename: r
                            .backend_metadata
                            .get("soulseek")
                            .and_then(|m| m.get("filename"))
                            .and_then(|v| v.as_str())
                            .unwrap_or(&r.filename)
                            .to_string(),
                        size: r.size,
                    },
                );
            }
        }
        Ok(results)
    }

    async fn stop(&self, id: &SearchId) -> Result<(), agpeer_common::Error> {
        let token = {
            let mut state = self.state.lock().unwrap();
            state.searches.remove(&id.to_string())
        };
        if let Some(token) = token {
            self.client.stop_search(token);
        }
        Ok(())
    }
}

#[async_trait]
impl agpeer_common::TransferBackend for NativeSoulseekBackend {
    fn backend(&self) -> Backend {
        Backend::Soulseek
    }

    async fn add(&self, request: AddTransferRequest) -> Result<Transfer, agpeer_common::Error> {
        if request.backend != Backend::Soulseek {
            return Err(agpeer_common::Error::InvalidSource);
        }
        const PREFIX: &str = "soulseek:result:";
        let result_key = request
            .source
            .strip_prefix(PREFIX)
            .ok_or(agpeer_common::Error::InvalidSource)?;

        let stored = {
            let state = self.state.lock().unwrap();
            state
                .results
                .get(result_key)
                .cloned()
                .ok_or(agpeer_common::Error::ResultExpired)?
        };

        let destination = request
            .destination
            .unwrap_or_else(|| self.config.download_dir.clone());

        // Kick off the native download (async; the F connection streams later).
        self.client
            .download(&stored.username, &stored.filename, stored.size.unwrap_or(0))
            .await
            .map_err(map_error)?;

        let metadata: HashMap<String, serde_json::Value> = serde_json::from_value(json!({
            "soulseek": {
                "username": &stored.username,
                "filename": &stored.filename,
                "size": stored.size,
            }
        }))
        .map_err(|e| agpeer_common::Error::Internal(e.to_string()))?;

        let transfer = Transfer {
            id: TransferId::new(),
            backend: Backend::Soulseek,
            source: request.source.clone(),
            display_name: request
                .display_name
                .unwrap_or_else(|| stored.filename.clone()),
            state: TransferState::Queued,
            progress: 0.0,
            bytes_total: stored.size,
            bytes_completed: 0,
            download_rate: None,
            upload_rate: None,
            eta: None,
            destination,
            created_at: chrono::Utc::now(),
            started_at: None,
            completed_at: None,
            error: None,
            files: Vec::new(),
            postprocess_state: agpeer_common::PostprocessState::None,
            metadata,
        };

        let mut state = self.state.lock().unwrap();
        state
            .transfers
            .insert(transfer.id.to_string(), transfer.clone());
        Ok(transfer)
    }

    async fn get(&self, id: &TransferId) -> Result<Transfer, agpeer_common::Error> {
        let mut state = self.state.lock().unwrap();
        let transfer = state
            .transfers
            .get_mut(&id.to_string())
            .ok_or(agpeer_common::Error::TransferNotFound)?;
        reconcile_transfer(transfer, &self.client.download_status());
        Ok(transfer.clone())
    }

    async fn list(&self) -> Result<Vec<Transfer>, agpeer_common::Error> {
        let status = self.client.download_status();
        let mut state = self.state.lock().unwrap();
        for transfer in state.transfers.values_mut() {
            reconcile_transfer(transfer, &status);
        }
        Ok(state.transfers.values().cloned().collect())
    }

    async fn pause(&self, _id: &TransferId) -> Result<(), agpeer_common::Error> {
        Err(agpeer_common::Error::InvalidState(
            "soulseek transfers cannot be paused in v1".into(),
        ))
    }

    async fn resume(&self, _id: &TransferId) -> Result<(), agpeer_common::Error> {
        Err(agpeer_common::Error::InvalidState(
            "soulseek transfers cannot be resumed in v1".into(),
        ))
    }

    async fn cancel(&self, id: &TransferId, delete_data: bool) -> Result<(), agpeer_common::Error> {
        let filename = {
            let mut state = self.state.lock().unwrap();
            let transfer = state
                .transfers
                .get_mut(&id.to_string())
                .ok_or(agpeer_common::Error::TransferNotFound)?;
            transfer.state = TransferState::Cancelled;
            transfer
                .metadata
                .get("soulseek")
                .and_then(|m| m.get("filename"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };
        if let Some(filename) = filename {
            self.client.cancel(&filename, delete_data);
        }
        Ok(())
    }

    async fn forget(&self, id: &TransferId) -> Result<(), agpeer_common::Error> {
        self.state.lock().unwrap().transfers.remove(&id.to_string());
        Ok(())
    }
}

/// Map a native [`rustsoseek::SearchResult`] into the shared application model,
/// deriving the application-owned opaque result id and backend metadata.
fn map_result(r: rustsoseek::SearchResult, search_id: SearchId) -> SearchResult {
    let (path, filename) = match r.filename.rsplit_once('/') {
        Some((dir, name)) => (dir.to_string(), name.to_string()),
        None => ("/".to_string(), r.filename.clone()),
    };

    let stable_key = format!("soulseek:{}:{}", r.username, r.filename);
    let result_id = agpeer_common::ResultId::from(uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        stable_key.as_bytes(),
    ));

    let mut backend_metadata = HashMap::new();
    backend_metadata.insert(
        "soulseek".to_string(),
        json!({
            "username": r.username,
            "filename": r.filename,
            "token": r.token,
        }),
    );

    SearchResult {
        result_id,
        search_id,
        username: r.username,
        path,
        filename,
        size: r.size,
        extension: r.extension,
        bitrate: r.bitrate,
        duration: r.duration,
        attributes: HashMap::new(),
        queue_length: r.queue_length,
        free_upload_slots: r.free_upload_slots,
        upload_speed: r.upload_speed,
        backend_metadata,
    }
}

/// Reconcile a transfer's progress/state from the native client's download
/// status (matched by username + filename to disambiguate same-name files from
/// different peers).
fn reconcile_transfer(transfer: &mut Transfer, status: &[rustsoseek::DownloadStatus]) {
    let soulseek = transfer.metadata.get("soulseek");
    let filename = soulseek
        .and_then(|m| m.get("filename"))
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let username = soulseek
        .and_then(|m| m.get("username"))
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let Some(dl) = status
        .iter()
        .find(|s| s.filename == filename && s.username == username)
    else {
        return;
    };
    transfer.bytes_total = Some(dl.size);
    transfer.bytes_completed = dl.offset;
    transfer.progress = if dl.size > 0 {
        (dl.offset as f32 / dl.size as f32).clamp(0.0, 1.0)
    } else {
        0.0
    };
    if dl.offset >= dl.size && dl.size > 0 {
        transfer.state = TransferState::Completed;
        if transfer.completed_at.is_none() {
            transfer.completed_at = Some(chrono::Utc::now());
        }
    } else if dl.offset > 0 {
        transfer.state = TransferState::Downloading;
    }
}
