//! Real librqbit-backed engine (compiled only behind the `rqbit` cargo
//! feature).
//!
//! # Status (honest, Phase 1)
//!
//! This engine is written against the librqbit **8.1.1** public API as
//! verified directly from the vendored crate source (`Session`,
//! `SessionOptions`, `AddTorrent`/`AddTorrentOptions`, `ManagedTorrent::stats`,
//! `TorrentStats`, `Session::delete`). It is feature-gated (off by default);
//! the default build, tests, and clippy use the in-memory reference engine.
//!
//! Behavior notes:
//!
//! - Magnets, local `.torrent` files, and remote `.torrent` URLs are all
//!   supported natively by rqbit (`AddTorrent::from_url` / `from_local_filename`).
//! - File selection is applied pre-download via `AddTorrentOptions::only_files`
//!   (numeric indices in torrent order); for magnets the indices only apply
//!   once metainfo resolves.
//! - PEX/LSD have no rqbit session toggle; rqbit disables peer exchange and
//!   peer discovery automatically for private torrents.
//! - `cancel` maps to `Session::delete` (job removed; data removed iff
//!   `delete_data`). The transfer record is dropped afterwards.
//! - Transfer ids are our opaque UUIDs; the rqbit `usize` ids stay in a
//!   private registry.

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::Arc;

use agpeer_common::{
    AddTransferRequest, Backend, PostprocessState, Transfer, TransferFile, TransferId,
    TransferState,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use librqbit::api::TorrentIdOrHash;
use librqbit::{
    AddTorrent, AddTorrentOptions, ManagedTorrent, Session, SessionOptions, TorrentStatsState,
};
use serde_json::json;
use tokio::sync::RwLock;

use crate::config::TorrentConfig;
use crate::engine::TorrentEngine;
use crate::error::BackendError;
use crate::normalize;
use crate::source::{self, SourceKind};

/// Registry entry for one transfer we added to the rqbit session.
struct RqbitEntry {
    id: TransferId,
    rqbit_id: usize,
    source: String,
    display_name: String,
    destination: String,
    created_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    metadata: HashMap<String, serde_json::Value>,
}

/// The librqbit-backed engine.
pub(crate) struct RqbitEngine {
    config: TorrentConfig,
    session: Arc<Session>,
    entries: RwLock<HashMap<TransferId, RqbitEntry>>,
}

impl RqbitEngine {
    pub(crate) async fn new(config: TorrentConfig) -> Result<Self, BackendError> {
        let opts = SessionOptions {
            disable_dht: !config.enable_dht,
            listen_port_range: config.listen_port.map(|port| port..port.saturating_add(1)),
            ratelimits: librqbit::limits::LimitsConfig {
                upload_bps: to_nonzero_u32(config.upload_rate_limit),
                download_bps: to_nonzero_u32(config.download_rate_limit),
            },
            ..Default::default()
        };

        let session = Session::new_with_opts(PathBuf::from(config.download_root.clone()), opts)
            .await
            .map_err(|e| BackendError::Internal(format!("failed to start rqbit session: {e:#}")))?;

        Ok(Self {
            config,
            session,
            entries: RwLock::new(HashMap::new()),
        })
    }
}

fn to_nonzero_u32(value: Option<u64>) -> Option<NonZeroU32> {
    value
        .and_then(|v| u32::try_from(v).ok())
        .and_then(NonZeroU32::new)
}

/// Build a normalized `Transfer` snapshot from an entry plus live rqbit state.
fn build_snapshot(entry: &mut RqbitEntry, handle: &Arc<ManagedTorrent>) -> Transfer {
    let stats = handle.stats();

    let is_paused = handle.is_paused();
    let state = match &stats.state {
        TorrentStatsState::Initializing if is_paused => TransferState::Queued,
        TorrentStatsState::Initializing => TransferState::Resolving,
        TorrentStatsState::Paused => TransferState::Paused,
        TorrentStatsState::Live if stats.finished => TransferState::Completed,
        TorrentStatsState::Live => TransferState::Downloading,
        TorrentStatsState::Error => TransferState::Failed,
    };
    if state == TransferState::Downloading && entry.started_at.is_none() {
        entry.started_at = Some(Utc::now());
    }
    if state == TransferState::Completed && entry.completed_at.is_none() {
        entry.completed_at = Some(Utc::now());
    }

    let total = stats.total_bytes;
    let completed = stats.progress_bytes.min(total);
    let progress = if total == 0 {
        if stats.finished {
            1.0
        } else {
            0.0
        }
    } else {
        completed as f32 / total as f32
    };

    let only_files = handle.only_files();
    let files: Vec<TransferFile> = handle
        .with_metadata(|m| {
            m.file_infos
                .iter()
                .enumerate()
                .map(|(idx, fi)| TransferFile {
                    index: idx.to_string(),
                    path: fi.relative_filename.to_string_lossy().into_owned(),
                    size: fi.len,
                    selected: only_files
                        .as_ref()
                        .map(|o| o.contains(&idx))
                        .unwrap_or(true),
                    bytes_completed: stats.file_progress.get(idx).copied().unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default();

    let (download_rate, upload_rate, peer_count, eta) = match stats.live.as_ref() {
        Some(live) => {
            let down = (live.download_speed.mbps * 1024.0 * 1024.0) as u64;
            let up = (live.upload_speed.mbps * 1024.0 * 1024.0) as u64;
            let peers = live.snapshot.peer_stats.live as u64;
            let eta = if !stats.finished && down > 0 && total > completed {
                Some((total - completed).div_ceil(down))
            } else {
                None
            };
            (Some(down), Some(up), peers, eta)
        }
        None => (Some(0), Some(0), 0, None),
    };

    let mut metadata = entry.metadata.clone();
    if let Some(serde_json::Value::Object(map)) = metadata.get_mut("torrent") {
        map.insert("peers".to_string(), json!(peer_count));
    }

    Transfer {
        id: entry.id,
        backend: Backend::Torrent,
        source: entry.source.clone(),
        display_name: entry.display_name.clone(),
        state,
        progress: progress.min(1.0),
        bytes_total: if total > 0 { Some(total) } else { None },
        bytes_completed: completed,
        download_rate,
        upload_rate,
        eta,
        destination: entry.destination.clone(),
        created_at: entry.created_at,
        started_at: entry.started_at,
        completed_at: entry.completed_at,
        error: stats.error.clone(),
        files,
        postprocess_state: PostprocessState::None,
        metadata,
    }
}

#[async_trait]
impl TorrentEngine for RqbitEngine {
    fn engine_name(&self) -> &'static str {
        "rqbit"
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

        let add = match &kind {
            SourceKind::Magnet { .. } | SourceKind::TorrentUrl(_) => {
                AddTorrent::from_url(request.source.clone())
            }
            SourceKind::TorrentFile(path) => {
                let path = path.to_str().ok_or(BackendError::InvalidSource)?;
                AddTorrent::from_local_filename(path).map_err(|e| {
                    BackendError::Internal(format!("failed to read torrent file: {e:#}"))
                })?
            }
        };

        let only_files = request
            .file_selection
            .as_ref()
            .map(|selection| {
                selection
                    .iter()
                    .filter(|s| s.selected)
                    .map(|s| {
                        s.index
                            .parse::<usize>()
                            .map_err(|_| BackendError::InvalidSource)
                    })
                    .collect::<Result<Vec<usize>, BackendError>>()
            })
            .transpose()?;

        let opts = AddTorrentOptions {
            output_folder: Some(destination.clone()),
            overwrite: true,
            only_files,
            disable_trackers: !self.config.enable_tracker,
            ..Default::default()
        };

        // librqbit 8.x `add_torrent` for a magnet resolves the metadata before
        // returning, so this can legitimately take a while for a cold DHT.
        // The timeout bounds the worst case (an unresolvable hash) instead of
        // hanging the API forever.
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(120),
            self.session.add_torrent(add, Some(opts)),
        )
        .await
        .map_err(|_| {
            BackendError::InvalidState(
                "rqbit add_torrent timed out (metadata not resolved in 120s)".to_string(),
            )
        })?
        .map_err(|e| BackendError::Unsupported(format!("rqbit rejected the source: {e:#}")))?;
        let handle = response
            .into_handle()
            .ok_or_else(|| BackendError::Internal("unexpected list-only response".to_string()))?;

        let id = TransferId::new();
        let created_at = Utc::now();
        let info_hash = handle.info_hash().as_string();
        let private = handle.with_metadata(|m| m.info.private).unwrap_or(false);
        let display_name = request
            .display_name
            .clone()
            .unwrap_or_else(|| handle.name().unwrap_or_else(|| "untitled".to_string()));

        let torrent = normalize::torrent_metadata("rqbit", Some(&info_hash), private);
        let metadata = normalize::merged_metadata(&request.metadata, torrent);

        let entry = RqbitEntry {
            id,
            rqbit_id: handle.id(),
            source: request.source.clone(),
            display_name,
            destination,
            created_at,
            started_at: None,
            completed_at: None,
            metadata,
        };
        self.entries.write().await.insert(id, entry);
        self.get(&id).await
    }

    async fn get(&self, id: &TransferId) -> Result<Transfer, BackendError> {
        let mut entries = self.entries.write().await;
        let entry = entries.get_mut(id).ok_or(BackendError::TransferNotFound)?;
        let handle = self
            .session
            .get(TorrentIdOrHash::Id(entry.rqbit_id))
            .ok_or(BackendError::TransferNotFound)?;
        Ok(build_snapshot(entry, &handle))
    }

    async fn list(&self) -> Result<Vec<Transfer>, BackendError> {
        let handles: Vec<Arc<ManagedTorrent>> = self
            .session
            .with_torrents(|iter| iter.map(|(_, handle)| handle.clone()).collect());

        let mut entries = self.entries.write().await;
        let mut out = Vec::with_capacity(handles.len());
        for handle in handles {
            let rqbit_id = handle.id();
            if let Some(entry) = entries.values_mut().find(|e| e.rqbit_id == rqbit_id) {
                out.push(build_snapshot(entry, &handle));
            }
        }
        Ok(out)
    }

    async fn pause(&self, id: &TransferId) -> Result<(), BackendError> {
        let entries = self.entries.read().await;
        let entry = entries.get(id).ok_or(BackendError::TransferNotFound)?;
        let handle = self
            .session
            .get(TorrentIdOrHash::Id(entry.rqbit_id))
            .ok_or(BackendError::TransferNotFound)?;
        self.session
            .pause(&handle)
            .await
            .map_err(|e| BackendError::InvalidState(format!("{e:#}")))
    }

    async fn resume(&self, id: &TransferId) -> Result<(), BackendError> {
        let entries = self.entries.read().await;
        let entry = entries.get(id).ok_or(BackendError::TransferNotFound)?;
        let handle = self
            .session
            .get(TorrentIdOrHash::Id(entry.rqbit_id))
            .ok_or(BackendError::TransferNotFound)?;
        self.session
            .unpause(&handle)
            .await
            .map_err(|e| BackendError::InvalidState(format!("{e:#}")))
    }

    async fn cancel(&self, id: &TransferId, delete_data: bool) -> Result<(), BackendError> {
        let entries = self.entries.read().await;
        let entry = entries.get(id).ok_or(BackendError::TransferNotFound)?;
        let rqbit_id = entry.rqbit_id;
        self.session
            .delete(TorrentIdOrHash::Id(rqbit_id), delete_data)
            .await
            .map_err(|e| BackendError::Internal(format!("{e:#}")))?;
        drop(entries);
        self.entries.write().await.remove(id);
        Ok(())
    }

    async fn forget(&self, id: &TransferId) -> Result<(), BackendError> {
        self.entries.write().await.remove(id);
        Ok(())
    }

    async fn shutdown(self: Box<Self>) -> Result<(), BackendError> {
        self.session.stop().await;
        Ok(())
    }
}
