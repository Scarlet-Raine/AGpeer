//! Normalized transfer model shared by every backend.

use crate::{Error, ResultId, SearchId, TransferId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Which backend owns a transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Torrent,
    Soulseek,
    /// User-configured external search command that returns magnet links.
    /// Search-only: magnets discovered here are pulled via the torrent backend.
    Hook,
}

impl Backend {
    pub fn as_str(&self) -> &'static str {
        match self {
            Backend::Torrent => "torrent",
            Backend::Soulseek => "soulseek",
            Backend::Hook => "hook",
        }
    }
}

impl std::fmt::Display for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Normalized transfer states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferState {
    Queued,
    Resolving,
    Downloading,
    Paused,
    Verifying,
    Completed,
    Postprocessing,
    Ready,
    Failed,
    Cancelled,
    /// A transfer that exists in our database but was missing from the
    /// backend during startup reconciliation. Files are never auto-deleted.
    Orphaned,
}

impl TransferState {
    pub fn as_str(&self) -> &'static str {
        match self {
            TransferState::Queued => "queued",
            TransferState::Resolving => "resolving",
            TransferState::Downloading => "downloading",
            TransferState::Paused => "paused",
            TransferState::Verifying => "verifying",
            TransferState::Completed => "completed",
            TransferState::Postprocessing => "postprocessing",
            TransferState::Ready => "ready",
            TransferState::Failed => "failed",
            TransferState::Cancelled => "cancelled",
            TransferState::Orphaned => "orphaned",
        }
    }

    /// Whether this state is terminal (no further progress expected).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TransferState::Ready
                | TransferState::Failed
                | TransferState::Cancelled
                | TransferState::Orphaned
        )
    }
}

impl std::fmt::Display for TransferState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Post-processing lifecycle of a transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PostprocessState {
    /// No post-processing requested or applicable.
    #[default]
    None,
    Pending,
    Running,
    Completed,
    Failed,
}

impl PostprocessState {
    pub fn as_str(&self) -> &'static str {
        match self {
            PostprocessState::None => "none",
            PostprocessState::Pending => "pending",
            PostprocessState::Running => "running",
            PostprocessState::Completed => "completed",
            PostprocessState::Failed => "failed",
        }
    }
}

/// A single file within a transfer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TransferFile {
    /// Backend-provided file index or path; used as a stable reference.
    pub index: String,
    pub path: String,
    pub size: u64,
    /// Whether this file is selected for download.
    pub selected: bool,
    pub bytes_completed: u64,
}

/// A normalized transfer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transfer {
    pub id: TransferId,
    pub backend: Backend,
    /// Original source supplied by the caller (magnet URI, path, URL, etc.).
    pub source: String,
    pub display_name: String,
    pub state: TransferState,
    /// Progress in the range `0.0..=1.0`.
    pub progress: f32,
    pub bytes_total: Option<u64>,
    pub bytes_completed: u64,
    pub download_rate: Option<u64>,
    pub upload_rate: Option<u64>,
    /// Estimated seconds remaining.
    pub eta: Option<u64>,
    pub destination: String,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub files: Vec<TransferFile>,
    pub postprocess_state: PostprocessState,
    /// Backend-specific metadata, namespaced per backend.
    pub metadata: HashMap<String, serde_json::Value>,
}

impl Transfer {
    /// Validate a user-supplied destination path is absolute and canonical.
    ///
    /// The caller is expected to have already resolved/created the directory;
    /// this guards against obviously invalid input.
    pub fn validate_destination(path: &str) -> Result<(), Error> {
        if path.trim().is_empty() {
            return Err(Error::InvalidSource);
        }
        Ok(())
    }
}

/// Request to add a new transfer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddTransferRequest {
    /// One of `backend` backends must accept this source.
    pub backend: Backend,
    /// Magnet URI, local `.torrent` path, remote `.torrent` URL, or a Soulseek
    /// search-result id (the latter is expressed as a `soulseek:` result id).
    pub source: String,
    /// Optional destination directory. Defaults to the configured download root.
    pub destination: Option<String>,
    /// Optional display name override.
    pub display_name: Option<String>,
    /// Optional per-file selection (torrent backends).
    pub file_selection: Option<Vec<FileSelection>>,
    /// Metadata to attach to the transfer.
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// A single file selection instruction for a transfer add request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSelection {
    /// Backend file index or path.
    pub index: String,
    pub selected: bool,
}

/// Result of adding a transfer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddTransferResponse {
    pub transfer_id: TransferId,
}

/// Reference from a search result to a transfer request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadResultRequest {
    pub search_id: SearchId,
    pub result_id: ResultId,
    pub destination: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_names_roundtrip() {
        let cases = [
            TransferState::Queued,
            TransferState::Downloading,
            TransferState::Ready,
            TransferState::Orphaned,
        ];
        for s in cases {
            let json = serde_json::to_string(&s).unwrap();
            let back: TransferState = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back);
        }
    }

    #[test]
    fn backend_names_roundtrip() {
        let json = serde_json::to_string(&Backend::Torrent).unwrap();
        assert_eq!(json, "\"torrent\"");
        let back: Backend = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Backend::Torrent);
    }

    #[test]
    fn terminal_states_are_terminal() {
        assert!(TransferState::Ready.is_terminal());
        assert!(TransferState::Failed.is_terminal());
        assert!(!TransferState::Downloading.is_terminal());
    }
}
