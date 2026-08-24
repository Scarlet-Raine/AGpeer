//! Shared application state: registered backends, config, database, event bus.

use crate::config::AppConfig;
use crate::event::EventBus;
use agpeer_common::{Backend, SearchBackend, TransferBackend};
use agpeer_storage::Database;
use chrono::{DateTime, Utc};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::watch;

/// Settings keys whose change requires the Soulseek backend to reconnect
/// (credentials, server address, or staging location).
pub const SOULSEEK_RELOAD_KEYS: &[&str] = &[
    "soulseek.username",
    "soulseek.password",
    "soulseek.server_addr",
    "soulseek.listen_port",
    "soulseek.download_root",
];

/// Application-wide shared state handed to every component.
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub db: Database,
    pub bus: EventBus,
    pub api_token: Arc<String>,
    pub started_at: DateTime<Utc>,
    /// Generation counter bumped whenever a backend-relevant setting changes;
    /// the Soulseek supervisor reconnects when it advances.
    backend_reload: watch::Sender<u64>,
    transfer_backends: RwLock<HashMap<Backend, Arc<dyn TransferBackend>>>,
    search_backends: RwLock<HashMap<Backend, Arc<dyn SearchBackend>>>,
}

impl AppState {
    pub fn new(config: AppConfig, db: Database, api_token: String) -> Arc<Self> {
        let (backend_reload, _) = watch::channel(0u64);
        Arc::new(Self {
            config: Arc::new(config),
            db,
            bus: EventBus::new(),
            api_token: Arc::new(api_token),
            started_at: Utc::now(),
            backend_reload,
            transfer_backends: RwLock::new(HashMap::new()),
            search_backends: RwLock::new(HashMap::new()),
        })
    }

    /// Subscribe to backend-reload generation changes.
    pub fn subscribe_backend_reload(&self) -> watch::Receiver<u64> {
        self.backend_reload.subscribe()
    }

    /// Bump the backend-reload generation if any of `changed_keys` requires
    /// it. Returns whether a reload was signaled.
    pub fn notify_settings_changed(&self, changed_keys: &[String]) -> bool {
        let needs_reload = changed_keys
            .iter()
            .any(|key| SOULSEEK_RELOAD_KEYS.contains(&key.as_str()));
        if needs_reload {
            self.backend_reload.send_modify(|gen| *gen += 1);
        }
        needs_reload
    }

    /// Register a transfer backend and publish its readiness.
    pub fn register_transfer_backend(&self, backend: Backend, engine: Arc<dyn TransferBackend>) {
        self.transfer_backends
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(backend, engine);
        self.bus
            .publish("backend.ready", json!({ "backend": backend.as_str() }));
    }

    /// Register a search backend.
    pub fn register_search_backend(&self, backend: Backend, engine: Arc<dyn SearchBackend>) {
        self.search_backends
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(backend, engine);
    }

    /// Look up a transfer backend by id.
    pub fn transfer_backend(&self, backend: Backend) -> Option<Arc<dyn TransferBackend>> {
        self.transfer_backends
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&backend)
            .cloned()
    }

    /// Look up a search backend by id.
    pub fn search_backend(&self, backend: Backend) -> Option<Arc<dyn SearchBackend>> {
        self.search_backends
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&backend)
            .cloned()
    }

    /// All registered transfer backends.
    pub fn all_transfer_backends(&self) -> Vec<(Backend, Arc<dyn TransferBackend>)> {
        self.transfer_backends
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(b, e)| (*b, e.clone()))
            .collect()
    }

    /// All registered search backends.
    pub fn all_search_backends(&self) -> Vec<(Backend, Arc<dyn SearchBackend>)> {
        self.search_backends
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(b, e)| (*b, e.clone()))
            .collect()
    }

    /// Union of every registered backend (transfer or search), sorted by name.
    pub fn available_backends(&self) -> Vec<Backend> {
        let mut backends: Vec<Backend> = Vec::new();
        for b in self
            .transfer_backends
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
        {
            backends.push(*b);
        }
        for b in self
            .search_backends
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
        {
            backends.push(*b);
        }
        backends.sort_by_key(|b| b.as_str());
        backends.dedup();
        backends
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agpeer_common::{
        AddTransferRequest, SearchId, SearchRequest, SearchResult, Transfer, TransferId,
    };
    use async_trait::async_trait;

    struct MockTransferBackend(Backend);

    #[async_trait]
    impl TransferBackend for MockTransferBackend {
        fn backend(&self) -> Backend {
            self.0
        }

        async fn add(&self, _request: AddTransferRequest) -> agpeer_common::Result<Transfer> {
            unimplemented!()
        }

        async fn get(&self, _id: &TransferId) -> agpeer_common::Result<Transfer> {
            unimplemented!()
        }

        async fn list(&self) -> agpeer_common::Result<Vec<Transfer>> {
            unimplemented!()
        }

        async fn pause(&self, _id: &TransferId) -> agpeer_common::Result<()> {
            unimplemented!()
        }

        async fn resume(&self, _id: &TransferId) -> agpeer_common::Result<()> {
            unimplemented!()
        }

        async fn cancel(&self, _id: &TransferId, _delete_data: bool) -> agpeer_common::Result<()> {
            unimplemented!()
        }

        async fn forget(&self, _id: &TransferId) -> agpeer_common::Result<()> {
            unimplemented!()
        }
    }

    struct MockSearchBackend(Backend);

    #[async_trait]
    impl SearchBackend for MockSearchBackend {
        fn backend(&self) -> Backend {
            self.0
        }

        async fn search(&self, _request: SearchRequest) -> agpeer_common::Result<SearchId> {
            unimplemented!()
        }

        async fn results(&self, _id: &SearchId) -> agpeer_common::Result<Vec<SearchResult>> {
            unimplemented!()
        }

        async fn stop(&self, _id: &SearchId) -> agpeer_common::Result<()> {
            unimplemented!()
        }
    }

    async fn mem_db() -> Database {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect in-memory db");
        let db = Database::from_pool(pool);
        db.migrate().await.expect("migrate");
        db
    }

    #[tokio::test]
    async fn register_and_lookup_backends() {
        let db = mem_db().await;
        let state = AppState::new(AppConfig::default(), db, "token".into());

        state.register_transfer_backend(
            Backend::Torrent,
            Arc::new(MockTransferBackend(Backend::Torrent)),
        );
        state.register_search_backend(
            Backend::Soulseek,
            Arc::new(MockSearchBackend(Backend::Soulseek)),
        );

        assert!(state.transfer_backend(Backend::Torrent).is_some());
        assert!(state.search_backend(Backend::Soulseek).is_some());
        assert!(state.transfer_backend(Backend::Soulseek).is_none());
        assert!(state.search_backend(Backend::Torrent).is_none());

        assert!(state
            .all_transfer_backends()
            .iter()
            .any(|(b, _)| *b == Backend::Torrent));
        assert!(state
            .all_search_backends()
            .iter()
            .any(|(b, _)| *b == Backend::Soulseek));

        let ready = state.bus.recent();
        assert!(ready.iter().any(|e| e.kind == "backend.ready"));
    }

    #[tokio::test]
    async fn available_backends_dedups_union() {
        let db = mem_db().await;
        let state = AppState::new(AppConfig::default(), db, "token".into());

        state.register_transfer_backend(
            Backend::Torrent,
            Arc::new(MockTransferBackend(Backend::Torrent)),
        );
        state.register_transfer_backend(
            Backend::Soulseek,
            Arc::new(MockTransferBackend(Backend::Soulseek)),
        );
        state.register_search_backend(
            Backend::Soulseek,
            Arc::new(MockSearchBackend(Backend::Soulseek)),
        );

        let backends = state.available_backends();
        assert_eq!(backends.len(), 2);
        // Sorted by backend name: "soulseek" < "torrent".
        assert_eq!(backends, vec![Backend::Soulseek, Backend::Torrent]);
    }
}
