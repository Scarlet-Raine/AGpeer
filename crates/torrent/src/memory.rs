//! The in-memory reference engine.
//!
//! **Reference/dev engine, not a real BitTorrent client.** It validates
//! sources, parses `.torrent` metainfo for names/sizes/private flags, tracks
//! normalized state, and simulates progress advancing over time at a fixed
//! rate. It is the default engine wired by [`crate::backend::TorrentBackend::new`].
//!
//! Simulation notes:
//!
//! - Progress advances at `config.download_rate_limit` bytes/second (or a
//!   fixed default). Time is measured with `tokio::time::Instant`, so tests
//!   using `#[tokio::test(start_paused = true)]` are fully deterministic.
//! - Magnet and remote-URL sources have no resolvable metainfo in a reference
//!   engine, so a synthetic single-file payload is fabricated (clearly marked
//!   `"simulated": true` in the metadata).
//! - `cancel` keeps a terminal `Cancelled` record in memory; `delete_data`
//!   zeroes the recorded per-file bytes and marks the transfer accordingly.

use std::collections::HashMap;

use agpeer_common::{
    AddTransferRequest, Backend, PostprocessState, Transfer, TransferFile, TransferId,
    TransferState,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::sync::RwLock;
use tokio::time::Instant;

use crate::bencode;
use crate::config::TorrentConfig;
use crate::engine::TorrentEngine;
use crate::error::BackendError;
use crate::normalize;
use crate::source::{self, SourceKind};

/// Rate used when `config.download_rate_limit` is unset.
const DEFAULT_SIMULATED_RATE_BPS: u64 = 512 * 1024;

/// Synthetic payload size for magnet/URL sources whose metainfo is unknown.
const SIMULATED_PAYLOAD_BYTES: u64 = 100 * 1024 * 1024;

/// State captured while a simulated transfer is downloading.
struct ResumePoint {
    started: Instant,
    bytes_at_start: u64,
    rate_bps: u64,
}

/// Everything the engine knows about one transfer.
struct MemoryEntry {
    id: TransferId,
    source: String,
    display_name: String,
    destination: String,
    state: TransferState,
    files: Vec<TransferFile>,
    created_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    error: Option<String>,
    metadata: HashMap<String, serde_json::Value>,
    resume: Option<ResumePoint>,
}

/// The in-memory reference engine.
pub(crate) struct MemoryEngine {
    config: TorrentConfig,
    transfers: RwLock<HashMap<TransferId, MemoryEntry>>,
}

impl MemoryEngine {
    pub(crate) fn new(config: TorrentConfig) -> Self {
        Self {
            config,
            transfers: RwLock::new(HashMap::new()),
        }
    }

    fn rate_bps(&self) -> u64 {
        self.config
            .download_rate_limit
            .unwrap_or(DEFAULT_SIMULATED_RATE_BPS)
    }

    /// Advance the simulated download clock for one entry.
    fn advance(entry: &mut MemoryEntry) {
        if entry.state != TransferState::Downloading {
            return;
        }
        let Some(resume) = entry.resume.as_ref() else {
            return;
        };
        let total = selected_bytes(&entry.files);
        if total == 0 {
            return;
        }

        let elapsed = resume.started.elapsed().as_secs_f64();
        let done = ((resume.bytes_at_start as f64) + elapsed * (resume.rate_bps as f64))
            .min(total as f64) as u64;
        let fraction = done as f64 / total as f64;

        for file in entry.files.iter_mut() {
            if file.selected {
                file.bytes_completed = ((fraction * file.size as f64) as u64).min(file.size);
            }
        }

        if done >= total {
            entry.state = TransferState::Completed;
            entry.completed_at = Some(Utc::now());
            entry.resume = None;
            for file in entry.files.iter_mut() {
                if file.selected {
                    file.bytes_completed = file.size;
                }
            }
        }
    }

    fn snapshot(entry: &MemoryEntry) -> Transfer {
        let total = selected_bytes(&entry.files);
        let completed = entry
            .files
            .iter()
            .filter(|f| f.selected)
            .map(|f| f.bytes_completed)
            .sum::<u64>();
        let progress = if total == 0 {
            0.0
        } else {
            completed as f32 / total as f32
        };

        let downloading = entry.state == TransferState::Downloading;
        let rate = entry.resume.as_ref().map(|r| r.rate_bps);
        let eta = match (downloading, rate) {
            (true, Some(rate)) if rate > 0 => Some((total - completed).div_ceil(rate)),
            _ => None,
        };

        Transfer {
            id: entry.id,
            backend: Backend::Torrent,
            source: entry.source.clone(),
            display_name: entry.display_name.clone(),
            state: entry.state,
            progress: progress.min(1.0),
            bytes_total: Some(total),
            bytes_completed: completed,
            download_rate: if downloading { rate } else { Some(0) },
            upload_rate: Some(0),
            eta,
            destination: entry.destination.clone(),
            created_at: entry.created_at,
            started_at: entry.started_at,
            completed_at: entry.completed_at,
            error: entry.error.clone(),
            files: entry.files.clone(),
            postprocess_state: PostprocessState::None,
            metadata: entry.metadata.clone(),
        }
    }
}

fn selected_bytes(files: &[TransferFile]) -> u64 {
    files
        .iter()
        .filter(|f| f.selected)
        .map(|f| f.size)
        .sum::<u64>()
}

/// Build the file list for a parsed `.torrent`.
fn files_from_metainfo(info: &bencode::TorrentInfo) -> Vec<TransferFile> {
    info.files
        .iter()
        .enumerate()
        .map(|(index, (path, size))| TransferFile {
            index: index.to_string(),
            path: path.clone(),
            size: *size,
            selected: true,
            bytes_completed: 0,
        })
        .collect()
}

/// Build the synthetic file list for sources with unknown metainfo.
fn synthetic_files(name: &str) -> Vec<TransferFile> {
    vec![TransferFile {
        index: "0".to_string(),
        path: format!("{name}.bin"),
        size: SIMULATED_PAYLOAD_BYTES,
        selected: true,
        bytes_completed: 0,
    }]
}

#[async_trait]
impl TorrentEngine for MemoryEngine {
    fn engine_name(&self) -> &'static str {
        "memory"
    }

    async fn add(&self, request: AddTransferRequest) -> Result<Transfer, BackendError> {
        if request.backend != Backend::Torrent {
            return Err(BackendError::Unsupported(format!(
                "unexpected backend: {}",
                request.backend
            )));
        }
        let kind = source::parse_source(&request.source)?;
        let destination = normalize::resolve_destination(&request, &self.config)?;
        let id = TransferId::new();
        let now = Utc::now();

        let (mut files, parsed_name, info_hash, private, simulated) = match &kind {
            SourceKind::Magnet { info_hash, .. } => {
                let fallback = source::fallback_name(&kind);
                (
                    synthetic_files(&fallback),
                    Some(fallback),
                    info_hash.clone(),
                    false,
                    true,
                )
            }
            SourceKind::TorrentUrl(_) => {
                let fallback = source::fallback_name(&kind);
                (
                    synthetic_files(&fallback),
                    Some(fallback),
                    None,
                    false,
                    true,
                )
            }
            SourceKind::TorrentFile(path) => {
                let bytes = std::fs::read(path).map_err(BackendError::Io)?;
                let Some(info) = bencode::torrent_info(&bytes) else {
                    return Err(BackendError::InvalidSource);
                };
                (
                    files_from_metainfo(&info),
                    Some(info.name.clone()),
                    None,
                    info.private,
                    false,
                )
            }
        };

        normalize::apply_file_selection(&mut files, request.file_selection.as_deref());

        let total = selected_bytes(&files);
        let (state, started_at, resume) = if total == 0 {
            (TransferState::Queued, None, None)
        } else {
            (
                TransferState::Downloading,
                Some(now),
                Some(ResumePoint {
                    started: Instant::now(),
                    bytes_at_start: 0,
                    rate_bps: self.rate_bps(),
                }),
            )
        };

        let display_name = request
            .display_name
            .clone()
            .or(parsed_name)
            .unwrap_or_else(|| "untitled".to_string());

        let mut torrent_meta = normalize::torrent_metadata("memory", info_hash.as_deref(), private);
        if simulated {
            if let serde_json::Value::Object(map) = &mut torrent_meta {
                map.insert("simulated".to_string(), serde_json::Value::Bool(true));
            }
        }
        let metadata = normalize::merged_metadata(&request.metadata, torrent_meta);

        let entry = MemoryEntry {
            id,
            source: request.source.clone(),
            display_name,
            destination,
            state,
            files,
            created_at: now,
            started_at,
            completed_at: None,
            error: None,
            metadata,
            resume,
        };

        self.transfers.write().await.insert(id, entry);
        self.get(&id).await
    }

    async fn get(&self, id: &TransferId) -> Result<Transfer, BackendError> {
        let mut transfers = self.transfers.write().await;
        let entry = transfers
            .get_mut(id)
            .ok_or(BackendError::TransferNotFound)?;
        Self::advance(entry);
        Ok(Self::snapshot(entry))
    }

    async fn list(&self) -> Result<Vec<Transfer>, BackendError> {
        let mut transfers = self.transfers.write().await;
        let mut out = Vec::with_capacity(transfers.len());
        for entry in transfers.values_mut() {
            Self::advance(entry);
            out.push(Self::snapshot(entry));
        }
        Ok(out)
    }

    async fn pause(&self, id: &TransferId) -> Result<(), BackendError> {
        let mut transfers = self.transfers.write().await;
        let entry = transfers
            .get_mut(id)
            .ok_or(BackendError::TransferNotFound)?;
        if entry.state != TransferState::Downloading {
            return Err(BackendError::InvalidState(format!(
                "cannot pause {}",
                entry.state
            )));
        }
        entry.state = TransferState::Paused;
        entry.resume = None;
        Ok(())
    }

    async fn resume(&self, id: &TransferId) -> Result<(), BackendError> {
        let mut transfers = self.transfers.write().await;
        let entry = transfers
            .get_mut(id)
            .ok_or(BackendError::TransferNotFound)?;
        if entry.state != TransferState::Paused {
            return Err(BackendError::InvalidState(format!(
                "cannot resume {}",
                entry.state
            )));
        }
        let bytes_at_start = entry
            .files
            .iter()
            .filter(|f| f.selected)
            .map(|f| f.bytes_completed)
            .sum::<u64>();
        if entry.started_at.is_none() {
            entry.started_at = Some(Utc::now());
        }
        entry.state = TransferState::Downloading;
        entry.resume = Some(ResumePoint {
            started: Instant::now(),
            bytes_at_start,
            rate_bps: self.rate_bps(),
        });
        Ok(())
    }

    async fn cancel(&self, id: &TransferId, delete_data: bool) -> Result<(), BackendError> {
        let mut transfers = self.transfers.write().await;
        let entry = transfers
            .get_mut(id)
            .ok_or(BackendError::TransferNotFound)?;
        match entry.state {
            TransferState::Cancelled => return Ok(()),
            TransferState::Ready | TransferState::Orphaned => {
                return Err(BackendError::InvalidState(format!(
                    "cannot cancel {}",
                    entry.state
                )));
            }
            _ => {}
        }
        entry.state = TransferState::Cancelled;
        entry.resume = None;
        if delete_data {
            for file in entry.files.iter_mut() {
                file.bytes_completed = 0;
            }
            if let Some(serde_json::Value::Object(map)) = entry.metadata.get_mut("torrent") {
                map.insert("data_deleted".to_string(), serde_json::Value::Bool(true));
            }
        }
        Ok(())
    }

    async fn forget(&self, id: &TransferId) -> Result<(), BackendError> {
        self.transfers.write().await.remove(id);
        Ok(())
    }

    async fn shutdown(self: Box<Self>) -> Result<(), BackendError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bencode::test_helpers::*;
    use crate::engine::TorrentEngine;
    use std::path::Path;
    use std::time::Duration;
    use tempfile::tempdir;

    const MAGNET: &str = "magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862&dn=Fixture";

    fn request(source: &str, destination: Option<&str>) -> AddTransferRequest {
        AddTransferRequest {
            backend: Backend::Torrent,
            source: source.to_string(),
            destination: destination.map(str::to_string),
            display_name: None,
            file_selection: None,
            metadata: HashMap::new(),
        }
    }

    fn write_fixture(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[tokio::test(start_paused = true)]
    async fn lifecycle_add_list_get_pause_resume_cancel_shutdown() {
        let dir = tempdir().unwrap();
        let config = TorrentConfig {
            download_root: dir.path().to_string_lossy().into_owned(),
            download_rate_limit: Some(1000),
            ..TorrentConfig::default()
        };
        let engine = MemoryEngine::new(config.clone());

        let added = engine.add(request(MAGNET, None)).await.unwrap();
        assert_eq!(added.state, TransferState::Downloading);
        assert_eq!(added.destination, config.download_root);
        assert_eq!(added.backend, Backend::Torrent);

        let list = engine.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, added.id);

        let got = engine.get(&added.id).await.unwrap();
        assert_eq!(got.id, added.id);
        assert_eq!(got.display_name, "Fixture");

        engine.pause(&added.id).await.unwrap();
        assert_eq!(
            engine.get(&added.id).await.unwrap().state,
            TransferState::Paused
        );
        // Progress freezes while paused.
        let before = engine.get(&added.id).await.unwrap().bytes_completed;
        tokio::time::sleep(Duration::from_secs(5)).await;
        let after = engine.get(&added.id).await.unwrap().bytes_completed;
        assert_eq!(before, after);

        engine.resume(&added.id).await.unwrap();
        assert_eq!(
            engine.get(&added.id).await.unwrap().state,
            TransferState::Downloading
        );

        engine.cancel(&added.id, false).await.unwrap();
        assert_eq!(
            engine.get(&added.id).await.unwrap().state,
            TransferState::Cancelled
        );

        // Cancel is idempotent.
        engine.cancel(&added.id, false).await.unwrap();

        Box::new(engine).shutdown().await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn simulated_progress_completes() {
        let dir = tempdir().unwrap();
        let torrent = write_fixture(
            dir.path(),
            "small.torrent",
            &torrent_metainfo_single_file("small", 1000, false),
        );
        let config = TorrentConfig {
            download_root: dir.path().to_string_lossy().into_owned(),
            download_rate_limit: Some(1000),
            ..TorrentConfig::default()
        };
        let engine = MemoryEngine::new(config);

        let added = engine
            .add(request(torrent.to_str().unwrap(), None))
            .await
            .unwrap();
        assert_eq!(added.state, TransferState::Downloading);
        assert_eq!(added.bytes_total, Some(1000));

        tokio::time::sleep(Duration::from_millis(500)).await;
        let mid = engine.get(&added.id).await.unwrap();
        assert!(mid.progress > 0.0 && mid.progress < 1.0);
        assert!(mid.download_rate.is_some());
        assert!(mid.eta.is_some());

        tokio::time::sleep(Duration::from_secs(3)).await;
        let done = engine.get(&added.id).await.unwrap();
        assert_eq!(done.state, TransferState::Completed);
        assert_eq!(done.progress, 1.0);
        assert_eq!(done.bytes_completed, 1000);
        assert!(done.completed_at.is_some());
        assert_eq!(done.files[0].bytes_completed, 1000);
    }

    #[tokio::test(start_paused = true)]
    async fn file_selection_is_applied() {
        let dir = tempdir().unwrap();
        let torrent = write_fixture(
            dir.path(),
            "multi.torrent",
            &torrent_metainfo_multi_file(&[("a.txt", 100), ("b.txt", 200)]),
        );
        let config = TorrentConfig {
            download_root: dir.path().to_string_lossy().into_owned(),
            download_rate_limit: Some(1000),
            ..TorrentConfig::default()
        };
        let engine = MemoryEngine::new(config);

        let mut req = request(torrent.to_str().unwrap(), None);
        req.file_selection = Some(vec![
            agpeer_common::FileSelection {
                index: "0".into(),
                selected: true,
            },
            agpeer_common::FileSelection {
                index: "1".into(),
                selected: false,
            },
        ]);
        let added = engine.add(req).await.unwrap();
        assert_eq!(added.files.len(), 2);
        assert!(added.files[0].selected);
        assert!(!added.files[1].selected);
        assert_eq!(added.bytes_total, Some(100));

        tokio::time::sleep(Duration::from_secs(5)).await;
        let done = engine.get(&added.id).await.unwrap();
        assert_eq!(done.state, TransferState::Completed);
        assert_eq!(done.bytes_completed, 100);
        assert_eq!(done.files[0].bytes_completed, 100);
        assert_eq!(done.files[1].bytes_completed, 0);
    }

    #[tokio::test]
    async fn private_torrent_is_recorded() {
        let dir = tempdir().unwrap();
        let private = write_fixture(
            dir.path(),
            "private.torrent",
            &torrent_metainfo_single_file("private", 100, true),
        );
        let public = write_fixture(
            dir.path(),
            "public.torrent",
            &torrent_metainfo_single_file("public", 100, false),
        );
        let config = TorrentConfig {
            download_root: dir.path().to_string_lossy().into_owned(),
            ..TorrentConfig::default()
        };
        let engine = MemoryEngine::new(config);

        let p = engine
            .add(request(private.to_str().unwrap(), None))
            .await
            .unwrap();
        let torrent_meta = p.metadata.get("torrent").unwrap();
        assert_eq!(torrent_meta["private"], serde_json::json!(true));
        assert_eq!(torrent_meta["engine"], "memory");

        let u = engine
            .add(request(public.to_str().unwrap(), None))
            .await
            .unwrap();
        assert!(u.metadata["torrent"].get("private").is_none());
    }

    #[tokio::test]
    async fn magnet_metadata_is_namespaced_and_simulated() {
        let dir = tempdir().unwrap();
        let config = TorrentConfig {
            download_root: dir.path().to_string_lossy().into_owned(),
            ..TorrentConfig::default()
        };
        let engine = MemoryEngine::new(config);
        let t = engine.add(request(MAGNET, None)).await.unwrap();
        let meta = t.metadata.get("torrent").unwrap();
        assert_eq!(meta["engine"], "memory");
        assert_eq!(
            meta["info_hash"],
            "cab507494d02ebb1178b38f2e9d7be299c86b862"
        );
        assert_eq!(meta["simulated"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn invalid_sources_are_rejected() {
        let dir = tempdir().unwrap();
        let config = TorrentConfig {
            download_root: dir.path().to_string_lossy().into_owned(),
            ..TorrentConfig::default()
        };
        let engine = MemoryEngine::new(config);

        for bad in [
            "not a magnet",
            "magnet:without-query",
            dir.path().join("nope.torrent").to_str().unwrap(),
        ] {
            let err = engine.add(request(bad, None)).await.unwrap_err();
            assert!(
                matches!(err, BackendError::InvalidSource),
                "{bad:?}: {err:?}"
            );
        }

        let garbage = write_fixture(dir.path(), "garbage.torrent", b"this is not bencode");
        let err = engine
            .add(request(garbage.to_str().unwrap(), None))
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::InvalidSource));
    }

    #[tokio::test]
    async fn missing_transfer_is_not_found() {
        let dir = tempdir().unwrap();
        let config = TorrentConfig {
            download_root: dir.path().to_string_lossy().into_owned(),
            ..TorrentConfig::default()
        };
        let engine = MemoryEngine::new(config);
        let id = TransferId::new();
        assert!(matches!(
            engine.get(&id).await,
            Err(BackendError::TransferNotFound)
        ));
        assert!(matches!(
            engine.pause(&id).await,
            Err(BackendError::TransferNotFound)
        ));
    }
}
