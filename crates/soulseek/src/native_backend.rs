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
use std::path::PathBuf;
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

    /// Drain peer refusals (`UploadFailed`) from the native client and mark
    /// the matching in-memory transfers failed, so a rejected download reports
    /// `Failed` with a readable error instead of sitting queued forever.
    fn absorb_failed_downloads(&self) {
        let failures = self.client.take_failed_downloads();
        if failures.is_empty() {
            return;
        }
        let mut state = self.state.lock().unwrap();
        apply_failed_transfers(&mut state.transfers, &failures);
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
        // Surface peer refusals before reconciling so a refused transfer
        // reports Failed rather than queued.
        self.absorb_failed_downloads();
        let mut state = self.state.lock().unwrap();
        let transfer = state
            .transfers
            .get_mut(&id.to_string())
            .ok_or(agpeer_common::Error::TransferNotFound)?;
        deliver_to_destination(transfer, &self.config.download_dir);
        reconcile_transfer(transfer, &self.client.download_status());
        Ok(transfer.clone())
    }

    async fn list(&self) -> Result<Vec<Transfer>, agpeer_common::Error> {
        self.absorb_failed_downloads();
        let status = self.client.download_status();
        let mut state = self.state.lock().unwrap();
        for transfer in state.transfers.values_mut() {
            deliver_to_destination(transfer, &self.config.download_dir);
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

/// Normalize a Soulseek path for comparison: peers mix separators between
/// search responses, queue requests, and refusal echoes.
fn normalize_separators(path: &str) -> String {
    path.replace('\\', "/")
}

/// Mark transfers matching drained `(username, filename)` peer refusals as
/// failed. Only live states are touched; paths compare separator-insensitively
/// because the peer echoes the refusal in its own share-index form.
fn apply_failed_transfers(
    transfers: &mut HashMap<String, Transfer>,
    failures: &[(String, String)],
) -> usize {
    let mut marked = 0;
    for (username, filename) in failures {
        let refused = normalize_separators(filename);
        for transfer in transfers.values_mut() {
            if transfer.state != TransferState::Queued
                && transfer.state != TransferState::Downloading
            {
                continue;
            }
            let Some(meta) = transfer.metadata.get("soulseek") else {
                continue;
            };
            let user_matches = meta.get("username").and_then(|v| v.as_str()) == Some(username);
            let file_matches = meta
                .get("filename")
                .and_then(|v| v.as_str())
                .map(|f| normalize_separators(f) == refused)
                .unwrap_or(false);
            if user_matches && file_matches {
                transfer.state = TransferState::Failed;
                transfer.error = Some("download refused by peer".to_string());
                marked += 1;
            }
        }
    }
    marked
}

/// Deliver a finished native download into the transfer's recorded
/// destination. The native client always streams into its configured
/// download root; when the caller supplied a different `destination`, move
/// the completed file there so completion checks (and the user) find it
/// where the transfer promised. Same-volume moves rename; cross-volume
/// delivery falls back to copy plus delete of our own completed artifact.
fn deliver_to_destination(transfer: &mut Transfer, download_dir: &str) {
    if transfer.metadata.get("delivered") == Some(&serde_json::Value::Bool(true)) {
        return;
    }
    let Some(expected) = transfer.bytes_total.filter(|b| *b > 0) else {
        return;
    };
    let filename = transfer
        .metadata
        .get("soulseek")
        .and_then(|m| m.get("filename"))
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if filename.is_empty() || transfer.destination.is_empty() {
        return;
    }
    if normalize_separators(&transfer.destination) == normalize_separators(download_dir) {
        return;
    }
    let basename = filename.rsplit(['/', '\\']).next().unwrap_or_default();
    if basename.is_empty() {
        return;
    }
    let src = PathBuf::from(download_dir).join(basename);
    let dst = PathBuf::from(&transfer.destination).join(basename);

    let src_len = std::fs::metadata(&src).map(|m| m.len()).ok();
    let dst_len = std::fs::metadata(&dst).map(|m| m.len()).ok();
    if dst_len == Some(expected) {
        // Already where it belongs (earlier delivery or an external move).
        transfer.metadata.insert("delivered".into(), json!(true));
        return;
    }
    if src_len != Some(expected) || dst.exists() {
        return;
    }
    std::fs::create_dir_all(&transfer.destination).ok();
    let moved = std::fs::rename(&src, &dst).is_ok()
        || (std::fs::copy(&src, &dst)
            .map(|n| n == expected)
            .unwrap_or(false)
            && std::fs::remove_file(&src).is_ok());
    if moved {
        transfer.metadata.insert("delivered".into(), json!(true));
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
        // No live status entry: the download may already have finished and the
        // driver no longer reports it (fast completions, restart, reconnect).
        // Verify from disk so a fully-downloaded file can never be stuck as a
        // queued/orphaned transfer. Any other case (peer never answered,
        // partial file) simply leaves the transfer untouched.
        if transfer.state != TransferState::Completed
            && transfer.state != TransferState::Cancelled
            && transfer.state != TransferState::Failed
        {
            if let Some(expected) = transfer.bytes_total.filter(|b| *b > 0) {
                let basename = filename.rsplit(['/', '\\']).next().unwrap_or_default();
                let path = PathBuf::from(&transfer.destination).join(basename);
                if std::fs::metadata(&path)
                    .map(|m| m.len() == expected)
                    .unwrap_or(false)
                {
                    transfer.bytes_completed = expected;
                    transfer.progress = 1.0;
                    transfer.state = TransferState::Completed;
                    if transfer.completed_at.is_none() {
                        transfer.completed_at = Some(chrono::Utc::now());
                    }
                }
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use agpeer_common::{Backend, PostprocessState, TransferId};
    use chrono::Utc;

    fn soulseek_transfer(destination: String, filename: &str, size: u64) -> Transfer {
        let metadata: HashMap<String, serde_json::Value> = serde_json::from_value(json!({
            "soulseek": { "username": "peer", "filename": filename, "size": size }
        }))
        .unwrap();
        Transfer {
            id: TransferId::new(),
            backend: Backend::Soulseek,
            source: "soulseek:result:test".into(),
            display_name: filename.into(),
            state: TransferState::Queued,
            progress: 0.0,
            bytes_total: Some(size),
            bytes_completed: 0,
            download_rate: None,
            upload_rate: None,
            eta: None,
            destination,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
            error: None,
            files: Vec::new(),
            postprocess_state: PostprocessState::None,
            metadata,
        }
    }

    #[test]
    fn disk_completion_is_detected_when_status_is_missing() {
        let base = std::env::temp_dir().join(format!("agpeer-slsk-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("song.flac"), vec![0u8; 1024]).unwrap();

        let mut t = soulseek_transfer(
            base.to_string_lossy().into_owned(),
            "Some\\Path\\song.flac",
            1024,
        );
        let status: Vec<rustsoseek::DownloadStatus> = Vec::new();

        reconcile_transfer(&mut t, &status);

        assert_eq!(t.state, TransferState::Completed);
        assert_eq!(t.bytes_completed, 1024);
        assert_eq!(t.progress, 1.0);
        assert!(t.completed_at.is_some());

        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn missing_or_partial_file_keeps_queued() {
        let base = std::env::temp_dir().join(format!("agpeer-slsk-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();

        // No file at all.
        let mut missing = soulseek_transfer(base.to_string_lossy().into_owned(), "gone.flac", 512);
        reconcile_transfer(&mut missing, &[]);
        assert_eq!(missing.state, TransferState::Queued);

        // Partial file (size mismatch) still does not complete.
        std::fs::write(base.join("partial.flac"), vec![0u8; 100]).unwrap();
        let mut partial =
            soulseek_transfer(base.to_string_lossy().into_owned(), "partial.flac", 1024);
        reconcile_transfer(&mut partial, &[]);
        assert_eq!(partial.state, TransferState::Queued);

        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn forward_slash_basenames_are_handled() {
        let base = std::env::temp_dir().join(format!("agpeer-slsk-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("song.flac"), vec![0u8; 2048]).unwrap();

        let mut t = soulseek_transfer(
            base.to_string_lossy().into_owned(),
            "peer/Music/song.flac",
            2048,
        );
        reconcile_transfer(&mut t, &[]);
        assert_eq!(t.state, TransferState::Completed);

        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn refused_download_marks_matching_transfer_failed() {
        let t = soulseek_transfer("dest".into(), "Some\\Path\\song.flac", 1024);
        let mut transfers = HashMap::new();
        transfers.insert(t.id.to_string(), t);

        // The peer echoes the refusal path with opposite separators.
        let marked = apply_failed_transfers(
            &mut transfers,
            &[("peer".to_string(), "Some/Path/song.flac".to_string())],
        );

        assert_eq!(marked, 1);
        let t = transfers.values().next().unwrap();
        assert_eq!(t.state, TransferState::Failed);
        assert_eq!(t.error.as_deref(), Some("download refused by peer"));
    }

    #[test]
    fn refusal_for_other_peer_or_terminal_transfer_is_ignored() {
        let a = soulseek_transfer("dest".into(), "song.flac", 10);
        let mut b = soulseek_transfer("dest".into(), "gone.flac", 5);
        b.state = TransferState::Completed;

        let mut transfers = HashMap::new();
        transfers.insert(a.id.to_string(), a);
        transfers.insert(b.id.to_string(), b);

        let marked = apply_failed_transfers(
            &mut transfers,
            &[("someoneelse".to_string(), "song.flac".to_string())],
        );
        assert_eq!(marked, 0);

        let marked = apply_failed_transfers(
            &mut transfers,
            &[("peer".to_string(), "gone.flac".to_string())],
        );
        assert_eq!(marked, 0);
        assert!(transfers.values().all(|t| t.state != TransferState::Failed));
    }

    #[test]
    fn completed_download_is_delivered_into_requested_destination() {
        let base = std::env::temp_dir().join(format!("agpeer-slsk-{}", uuid::Uuid::new_v4()));
        let dl = base.join("dl");
        let dest = base.join("unsorted");
        std::fs::create_dir_all(&dl).unwrap();
        std::fs::write(dl.join("song.ogg"), vec![0u8; 100]).unwrap();

        let mut t = soulseek_transfer(dest.to_string_lossy().into_owned(), "a\\b\\song.ogg", 100);
        deliver_to_destination(&mut t, dl.to_str().unwrap());

        assert_eq!(
            std::fs::metadata(dest.join("song.ogg"))
                .map(|m| m.len())
                .unwrap(),
            100
        );
        assert!(!dl.join("song.ogg").exists());
        assert_eq!(t.metadata.get("delivered"), Some(&json!(true)));

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn partial_source_is_not_delivered() {
        let base = std::env::temp_dir().join(format!("agpeer-slsk-{}", uuid::Uuid::new_v4()));
        let dl = base.join("dl");
        let dest = base.join("unsorted");
        std::fs::create_dir_all(&dl).unwrap();
        std::fs::write(dl.join("song.ogg"), vec![0u8; 50]).unwrap();

        let mut t = soulseek_transfer(dest.to_string_lossy().into_owned(), "song.ogg", 100);
        deliver_to_destination(&mut t, dl.to_str().unwrap());

        assert!(!dest.join("song.ogg").exists());
        assert!(!t.metadata.contains_key("delivered"));

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn same_destination_is_a_noop() {
        let base = std::env::temp_dir().join(format!("agpeer-slsk-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("song.ogg"), vec![0u8; 100]).unwrap();

        let mut t = soulseek_transfer(base.to_string_lossy().into_owned(), "song.ogg", 100);
        deliver_to_destination(&mut t, base.to_str().unwrap());

        // File untouched in place; no delivery bookkeeping.
        assert!(base.join("song.ogg").exists());
        assert!(!t.metadata.contains_key("delivered"));

        std::fs::remove_dir_all(&base).ok();
    }
}
