//! Search model (Soulseek-oriented but backend-neutral in shape).

use crate::{ResultId, SearchId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Lifecycle of a search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchState {
    Pending,
    Active,
    Completed,
    Failed,
    Expired,
    Stopped,
}

impl SearchState {
    pub fn as_str(&self) -> &'static str {
        match self {
            SearchState::Pending => "pending",
            SearchState::Active => "active",
            SearchState::Completed => "completed",
            SearchState::Failed => "failed",
            SearchState::Expired => "expired",
            SearchState::Stopped => "stopped",
        }
    }
}

/// Request to start a search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRequest {
    pub backend: crate::Backend,
    pub query: String,
    /// Optional Soulseek user to restrict results to.
    pub user: Option<String>,
    /// Optional extension filter, e.g. `flac`.
    pub extension: Option<String>,
    /// Minimum size in bytes.
    pub min_size: Option<u64>,
    /// Maximum results to retain.
    #[serde(default = "default_max_results")]
    pub max_results: usize,
}

fn default_max_results() -> usize {
    1000
}

/// A single search result with an application-generated opaque id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub result_id: ResultId,
    pub search_id: SearchId,
    pub username: String,
    pub path: String,
    pub filename: String,
    pub size: Option<u64>,
    pub extension: Option<String>,
    pub bitrate: Option<u32>,
    pub duration: Option<u32>,
    pub attributes: HashMap<String, serde_json::Value>,
    pub queue_length: Option<u32>,
    pub free_upload_slots: Option<bool>,
    pub upload_speed: Option<u64>,
    pub backend_metadata: HashMap<String, serde_json::Value>,
}

/// A search as persisted/returned by the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Search {
    pub id: SearchId,
    pub backend: crate::Backend,
    pub query: String,
    pub state: SearchState,
    pub result_count: usize,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}
