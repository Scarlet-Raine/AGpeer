use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use agpeer_common::{
    AddTransferRequest, AddTransferResponse, Backend, Error, ResultId, Search, SearchId,
    SearchResult, SearchState, Transfer, TransferFile, TransferId, TransferState,
};
use agpeer_core::state::AppState;
use agpeer_storage::{JobStore, SearchStore, SettingsStore, TransferStore};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Sse;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde_json::json;
use utoipa::OpenApi;

use crate::auth::BearerAuth;
use crate::dto::{
    err_to_response, ApiErrorResponse, BackendStatus, CancelRequest, DownloadDestination,
    MessageResponse, SearchResponse, StatusResponse,
};
use crate::sse::sse_stream;

/// Effective runtime-enabled state of the hook (magnet search) backend. The
/// WebUI toggles `hook_search.enabled` via the settings API; the static config
/// value is only the initial seed.
async fn hook_search_enabled(state: &Arc<AppState>) -> bool {
    SettingsStore::new(&state.db)
        .get_typed::<bool>("hook_search.enabled")
        .await
        .ok()
        .flatten()
        .unwrap_or(state.config.hook_search.enabled)
}

pub async fn status(
    State(state): State<Arc<AppState>>,
    _auth: BearerAuth,
) -> Result<Json<StatusResponse>, ApiErrorResponse> {
    let uptime_secs = (Utc::now() - state.started_at).num_seconds().max(0) as u64;
    let hook_enabled = hook_search_enabled(&state).await;
    let backends = state
        .available_backends()
        .into_iter()
        .map(|b| BackendStatus {
            backend: b.as_str().to_string(),
            transfer_available: state.transfer_backend(b).is_some(),
            search_available: match b {
                Backend::Hook => hook_enabled,
                _ => state.search_backend(b).is_some(),
            },
            state: "ready".into(),
        })
        .collect();
    Ok(Json(StatusResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_secs,
        server_time: Utc::now(),
        db: if state.db.pool().is_closed() {
            "closed".to_string()
        } else {
            "ok".to_string()
        },
        backends,
    }))
}

pub async fn backends(
    State(state): State<Arc<AppState>>,
    _auth: BearerAuth,
) -> Result<Json<Vec<BackendStatus>>, ApiErrorResponse> {
    let hook_enabled = hook_search_enabled(&state).await;
    // List only registered backends so callers can distinguish "not registered
    // (e.g. no hook command configured)" from "registered but disabled".
    let backends = state
        .available_backends()
        .into_iter()
        .map(|b| BackendStatus {
            backend: b.as_str().to_string(),
            transfer_available: state.transfer_backend(b).is_some(),
            search_available: match b {
                Backend::Hook => hook_enabled && state.search_backend(b).is_some(),
                _ => state.search_backend(b).is_some(),
            },
            state: "ready".into(),
        })
        .collect();
    Ok(Json(backends))
}

pub async fn events(
    State(state): State<Arc<AppState>>,
    _auth: BearerAuth,
) -> Sse<impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>>
{
    sse_stream(state.bus.clone())
}

pub async fn list_transfers(
    State(state): State<Arc<AppState>>,
    _auth: BearerAuth,
) -> Result<Json<Vec<Transfer>>, ApiErrorResponse> {
    let transfers = TransferStore::new(&state.db)
        .list()
        .await
        .map_err(err_to_response)?;
    Ok(Json(transfers))
}

pub async fn add_transfer(
    State(state): State<Arc<AppState>>,
    _auth: BearerAuth,
    Json(req): Json<AddTransferRequest>,
) -> Result<(StatusCode, Json<AddTransferResponse>), ApiErrorResponse> {
    let backend = state
        .transfer_backend(req.backend)
        .ok_or_else(|| err_to_response(Error::BackendUnavailable))?;
    let transfer = backend.add(req).await.map_err(err_to_response)?;
    let store = TransferStore::new(&state.db);
    store.upsert(&transfer).await.map_err(err_to_response)?;
    store
        .replace_files(&transfer.id, &transfer.files)
        .await
        .map_err(err_to_response)?;
    state.bus.publish(
        "transfer.added",
        json!({"id": transfer.id, "backend": transfer.backend.as_str(), "display_name": transfer.display_name}),
    );
    Ok((
        StatusCode::CREATED,
        Json(AddTransferResponse {
            transfer_id: transfer.id,
        }),
    ))
}

pub async fn get_transfer(
    State(state): State<Arc<AppState>>,
    _auth: BearerAuth,
    Path(id): Path<TransferId>,
) -> Result<Json<Transfer>, ApiErrorResponse> {
    let transfer = TransferStore::new(&state.db)
        .get(&id)
        .await
        .map_err(err_to_response)?
        .ok_or_else(|| err_to_response(Error::TransferNotFound))?;
    Ok(Json(transfer))
}

pub async fn delete_transfer(
    State(state): State<Arc<AppState>>,
    _auth: BearerAuth,
    Path(id): Path<TransferId>,
    Json(req): Json<CancelRequest>,
) -> Result<Json<MessageResponse>, ApiErrorResponse> {
    let delete_data = req.delete_data.unwrap_or(false);
    let store = TransferStore::new(&state.db);
    let transfer = store
        .get(&id)
        .await
        .map_err(err_to_response)?
        .ok_or_else(|| err_to_response(Error::TransferNotFound))?;
    if let Some(backend) = state.transfer_backend(transfer.backend) {
        let _ = backend.cancel(&id, delete_data).await;
        // Drop the transfer from the backend registry too, otherwise the
        // periodic reconcile re-imports it ("transfer pops back up").
        let _ = backend.forget(&id).await;
    }
    if delete_data {
        delete_transfer_files(&transfer);
    }
    store.delete(&id).await.map_err(err_to_response)?;
    state.bus.publish("transfer.removed", json!({"id": id}));
    Ok(Json(MessageResponse {
        message: "transfer removed".into(),
    }))
}

/// Delete the downloaded files of `transfer`, constrained within its
/// destination root. Explicitly authorized via `delete_data`.
fn delete_transfer_files(transfer: &Transfer) {
    let root = FsPath::new(&transfer.destination);
    let Ok(root_meta) = root.canonicalize() else {
        return;
    };
    if !root_meta.is_dir() {
        return;
    }
    let mut removed: Vec<PathBuf> = Vec::new();
    for file in &transfer.files {
        let target = FsPath::new(&file.path);
        let target = if target.is_absolute() {
            target.to_path_buf()
        } else {
            root.join(target)
        };
        // Resolve and refuse anything that escapes the destination root.
        let Ok(canon) = target.canonicalize() else {
            // Recorded path not on disk (e.g. a transfer that was never fully
            // reconciled): find the file by name anywhere under the root.
            if let Some(name) = target.file_name() {
                if let Some(found) = find_by_name(&root_meta, &name.to_string_lossy()) {
                    removed.push(found);
                }
            }
            continue;
        };
        if !canon.starts_with(&root_meta) {
            continue;
        }
        if canon.is_file() && std::fs::remove_file(&canon).is_ok() {
            removed.push(canon);
        }
    }
    // Prune now-empty ancestor directories, stopping at the root.
    for file in removed {
        let mut parent = file.parent();
        while let Some(p) = parent {
            if p == root_meta {
                break;
            }
            let mut entries = match std::fs::read_dir(p) {
                Ok(e) => e,
                Err(_) => break,
            };
            if entries.next().is_some() {
                break;
            }
            if std::fs::remove_dir(p).is_err() {
                break;
            }
            parent = p.parent();
        }
    }
}

/// Recursively delete the first file named `target` under `dir`, returning its
/// (absolute) path. Used as a fallback when a transfer's recorded path is
/// stale or points elsewhere in the staging tree.
fn find_by_name(dir: &FsPath, target: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if let Some(found) = find_by_name(&p, target) {
                return Some(found);
            }
        } else if p.is_file()
            && entry.file_name().to_string_lossy() == target
            && std::fs::remove_file(&p).is_ok()
        {
            return Some(p);
        }
    }
    None
}

pub async fn pause_transfer(
    State(state): State<Arc<AppState>>,
    _auth: BearerAuth,
    Path(id): Path<TransferId>,
) -> Result<Json<Transfer>, ApiErrorResponse> {
    let transfer = set_paused(&state, &id, true).await?;
    Ok(Json(transfer))
}

pub async fn resume_transfer(
    State(state): State<Arc<AppState>>,
    _auth: BearerAuth,
    Path(id): Path<TransferId>,
) -> Result<Json<Transfer>, ApiErrorResponse> {
    let transfer = set_paused(&state, &id, false).await?;
    Ok(Json(transfer))
}

async fn set_paused(
    state: &Arc<AppState>,
    id: &TransferId,
    paused: bool,
) -> Result<Transfer, ApiErrorResponse> {
    let store = TransferStore::new(&state.db);
    let transfer = store
        .get(id)
        .await
        .map_err(err_to_response)?
        .ok_or_else(|| err_to_response(Error::TransferNotFound))?;
    let backend = state
        .transfer_backend(transfer.backend)
        .ok_or_else(|| err_to_response(Error::BackendUnavailable))?;
    if paused {
        backend.pause(id).await.map_err(err_to_response)?;
    } else {
        backend.resume(id).await.map_err(err_to_response)?;
    }
    let updated = backend.get(id).await.map_err(err_to_response)?;
    store.upsert(&updated).await.map_err(err_to_response)?;
    store
        .replace_files(&updated.id, &updated.files)
        .await
        .map_err(err_to_response)?;
    state.bus.publish(
        if paused {
            "transfer.paused"
        } else {
            "transfer.resumed"
        },
        json!({"id": id, "state": updated.state.as_str()}),
    );
    Ok(updated)
}

pub async fn cancel_transfer(
    State(state): State<Arc<AppState>>,
    _auth: BearerAuth,
    Path(id): Path<TransferId>,
    Json(req): Json<CancelRequest>,
) -> Result<Json<Transfer>, ApiErrorResponse> {
    let store = TransferStore::new(&state.db);
    let mut transfer = store
        .get(&id)
        .await
        .map_err(err_to_response)?
        .ok_or_else(|| err_to_response(Error::TransferNotFound))?;
    if let Some(backend) = state.transfer_backend(transfer.backend) {
        let _ = backend.cancel(&id, req.delete_data.unwrap_or(false)).await;
    }
    transfer.state = TransferState::Cancelled;
    store.upsert(&transfer).await.map_err(err_to_response)?;
    state.bus.publish("transfer.cancelled", json!({"id": id}));
    Ok(Json(transfer))
}

pub async fn transfer_files(
    State(state): State<Arc<AppState>>,
    _auth: BearerAuth,
    Path(id): Path<TransferId>,
) -> Result<Json<Vec<TransferFile>>, ApiErrorResponse> {
    let store = TransferStore::new(&state.db);
    let _ = store
        .get(&id)
        .await
        .map_err(err_to_response)?
        .ok_or_else(|| err_to_response(Error::TransferNotFound))?;
    let files = store.files(&id).await.map_err(err_to_response)?;
    Ok(Json(files))
}

pub async fn list_searches(
    State(state): State<Arc<AppState>>,
    _auth: BearerAuth,
) -> Result<Json<Vec<Search>>, ApiErrorResponse> {
    let searches = SearchStore::new(&state.db)
        .list()
        .await
        .map_err(err_to_response)?;
    Ok(Json(searches))
}

pub async fn add_search(
    State(state): State<Arc<AppState>>,
    _auth: BearerAuth,
    Json(req): Json<agpeer_common::SearchRequest>,
) -> Result<(StatusCode, Json<SearchResponse>), ApiErrorResponse> {
    let backend = state
        .search_backend(req.backend)
        .ok_or_else(|| err_to_response(Error::BackendUnavailable))?;
    // The hook backend enforces its own runtime `enabled` toggle (settings
    // table) and returns BackendUnavailable (503) when disabled.
    let id = backend.search(req.clone()).await.map_err(err_to_response)?;
    let now = Utc::now();
    let search = Search {
        id,
        backend: req.backend,
        query: req.query.clone(),
        state: SearchState::Active,
        result_count: 0,
        created_at: now,
        expires_at: now + chrono::Duration::hours(state.config.search_result_ttl_hours as i64),
    };
    SearchStore::new(&state.db)
        .upsert(&search)
        .await
        .map_err(err_to_response)?;
    state.bus.publish(
        "search.started",
        json!({"search_id": id, "query": req.query}),
    );
    Ok((
        StatusCode::CREATED,
        Json(SearchResponse {
            search_id: id.to_string(),
        }),
    ))
}

pub async fn get_search(
    State(state): State<Arc<AppState>>,
    _auth: BearerAuth,
    Path(id): Path<SearchId>,
) -> Result<Json<Search>, ApiErrorResponse> {
    let search = SearchStore::new(&state.db)
        .get(&id)
        .await
        .map_err(err_to_response)?
        .ok_or_else(|| err_to_response(Error::SearchNotFound))?;
    Ok(Json(search))
}

pub async fn search_results(
    State(state): State<Arc<AppState>>,
    _auth: BearerAuth,
    Path(id): Path<SearchId>,
) -> Result<Json<Vec<SearchResult>>, ApiErrorResponse> {
    let store = SearchStore::new(&state.db);
    let search = store
        .get(&id)
        .await
        .map_err(err_to_response)?
        .ok_or_else(|| err_to_response(Error::SearchNotFound))?;

    // Results live in the backend (the soulseek adapter accumulates them in
    // memory). Prefer the live backend; fall back to whatever is persisted if
    // the backend is no longer available. Results are capped by the backend to
    // `max_results`, and DB persistence is intentionally skipped here so a
    // large result set (tens of thousands of rows) can never make this
    // endpoint slow.
    let results = match state.search_backend(search.backend) {
        Some(backend) => backend.results(&id).await.map_err(err_to_response)?,
        None => store.results(&id).await.map_err(err_to_response)?,
    };

    Ok(Json(results))
}

pub async fn stop_search(
    State(state): State<Arc<AppState>>,
    _auth: BearerAuth,
    Path(id): Path<SearchId>,
) -> Result<Json<MessageResponse>, ApiErrorResponse> {
    let store = SearchStore::new(&state.db);
    let mut search = store
        .get(&id)
        .await
        .map_err(err_to_response)?
        .ok_or_else(|| err_to_response(Error::SearchNotFound))?;
    if let Some(backend) = state.search_backend(search.backend) {
        backend.stop(&id).await.map_err(err_to_response)?;
    }
    search.state = SearchState::Stopped;
    store.upsert(&search).await.map_err(err_to_response)?;
    state
        .bus
        .publish("search.stopped", json!({"search_id": id}));
    Ok(Json(MessageResponse {
        message: "search stopped".into(),
    }))
}

pub async fn download_result(
    State(state): State<Arc<AppState>>,
    _auth: BearerAuth,
    Path(ids): Path<(SearchId, ResultId)>,
    body: Option<Json<DownloadDestination>>,
) -> Result<(StatusCode, Json<AddTransferResponse>), ApiErrorResponse> {
    let (search_id, result_id) = ids;

    // Results are held by the search backend (in memory), not the database.
    // Resolve the target from the live result list so a "download" always
    // finds the file the UI is pointing at.
    let search_backend = state
        .search_backend(Backend::Soulseek)
        .ok_or_else(|| err_to_response(Error::BackendUnavailable))?;
    let results = search_backend
        .results(&search_id)
        .await
        .map_err(err_to_response)?;
    let result = results
        .into_iter()
        .find(|r| r.result_id == result_id && r.search_id == search_id)
        .ok_or_else(|| err_to_response(Error::ResultNotFound))?;

    let destination = body.and_then(|b| b.0.destination);
    let add_req = AddTransferRequest {
        backend: Backend::Soulseek,
        source: format!("soulseek:result:{}", result_id),
        destination,
        display_name: Some(result.filename.clone()),
        file_selection: None,
        metadata: Default::default(),
    };
    let backend = state
        .transfer_backend(Backend::Soulseek)
        .ok_or_else(|| err_to_response(Error::BackendUnavailable))?;
    let transfer = backend.add(add_req).await.map_err(err_to_response)?;
    let tstore = TransferStore::new(&state.db);
    tstore.upsert(&transfer).await.map_err(err_to_response)?;
    tstore
        .replace_files(&transfer.id, &transfer.files)
        .await
        .map_err(err_to_response)?;
    state.bus.publish(
        "transfer.added",
        json!({"id": transfer.id, "backend": transfer.backend.as_str(), "display_name": transfer.display_name}),
    );
    Ok((
        StatusCode::CREATED,
        Json(AddTransferResponse {
            transfer_id: transfer.id,
        }),
    ))
}

pub async fn get_settings(
    State(state): State<Arc<AppState>>,
    _auth: BearerAuth,
) -> Result<Json<serde_json::Value>, ApiErrorResponse> {
    let all = SettingsStore::new(&state.db)
        .all()
        .await
        .map_err(err_to_response)?;
    Ok(Json(serde_json::Value::Object(all)))
}

pub async fn put_settings(
    State(state): State<Arc<AppState>>,
    _auth: BearerAuth,
    Json(map): Json<serde_json::Map<String, serde_json::Value>>,
) -> Result<Json<serde_json::Value>, ApiErrorResponse> {
    let store = SettingsStore::new(&state.db);
    for (key, value) in map {
        store.set(&key, &value).await.map_err(err_to_response)?;
    }
    let all = store.all().await.map_err(err_to_response)?;
    Ok(Json(serde_json::Value::Object(all)))
}

pub async fn get_setting(
    State(state): State<Arc<AppState>>,
    _auth: BearerAuth,
    Path(key): Path<String>,
) -> Result<Json<serde_json::Value>, ApiErrorResponse> {
    let value = SettingsStore::new(&state.db)
        .get(&key)
        .await
        .map_err(err_to_response)?
        .ok_or_else(|| err_to_response(Error::NotFound))?;
    Ok(Json(value))
}

pub async fn put_setting(
    State(state): State<Arc<AppState>>,
    _auth: BearerAuth,
    Path(key): Path<String>,
    Json(value): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiErrorResponse> {
    SettingsStore::new(&state.db)
        .set(&key, &value)
        .await
        .map_err(err_to_response)?;
    Ok(Json(value))
}

pub async fn delete_setting(
    State(state): State<Arc<AppState>>,
    _auth: BearerAuth,
    Path(key): Path<String>,
) -> Result<Json<MessageResponse>, ApiErrorResponse> {
    SettingsStore::new(&state.db)
        .delete(&key)
        .await
        .map_err(err_to_response)?;
    Ok(Json(MessageResponse {
        message: "setting deleted".into(),
    }))
}

pub async fn list_postprocess(
    State(state): State<Arc<AppState>>,
    _auth: BearerAuth,
) -> Result<Json<Vec<agpeer_jobs::Job>>, ApiErrorResponse> {
    let jobs = JobStore::new(&state.db)
        .list()
        .await
        .map_err(err_to_response)?;
    Ok(Json(jobs))
}

pub async fn get_postprocess(
    State(state): State<Arc<AppState>>,
    _auth: BearerAuth,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<agpeer_jobs::Job>, ApiErrorResponse> {
    let job = JobStore::new(&state.db)
        .get(&id)
        .await
        .map_err(err_to_response)?
        .ok_or_else(|| err_to_response(Error::NotFound))?;
    Ok(Json(job))
}

/// A single file/directory in the organized media library.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LibraryEntry {
    /// Path relative to the library root, e.g. `TV Shows/Show Name/Season 01/ep.mkv`.
    pub path: String,
    /// Absolute path on disk (for opening the folder in the OS file manager).
    pub absolute_path: String,
    pub size: Option<u64>,
    pub is_dir: bool,
}

fn walk_library(dir: &FsPath, root: &FsPath, out: &mut Vec<LibraryEntry>) -> Result<(), Error> {
    let entries = std::fs::read_dir(dir).map_err(|e| Error::Internal(e.to_string()))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::Internal(e.to_string()))?;
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        let absolute = path.to_string_lossy().into_owned();
        let metadata = entry
            .metadata()
            .map_err(|e| Error::Internal(e.to_string()))?;
        if metadata.is_dir() {
            out.push(LibraryEntry {
                path: rel,
                absolute_path: absolute,
                size: None,
                is_dir: true,
            });
            walk_library(&path, root, out)?;
        } else {
            out.push(LibraryEntry {
                path: rel,
                absolute_path: absolute,
                size: Some(metadata.len()),
                is_dir: false,
            });
        }
    }
    Ok(())
}

/// List the organized media library (files under `[postprocess].library_root`).
pub async fn list_library(
    State(state): State<Arc<AppState>>,
    _auth: BearerAuth,
) -> Result<Json<Vec<LibraryEntry>>, ApiErrorResponse> {
    let root = PathBuf::from(&state.config.postprocess.library_root);
    if root.as_os_str().is_empty() || !root.is_dir() {
        return Ok(Json(Vec::new()));
    }
    let mut out = Vec::new();
    walk_library(&root, &root, &mut out).map_err(err_to_response)?;
    // Directories first, then by path.
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.path.cmp(&b.path)));
    Ok(Json(out))
}

/// Router for the agpeer API.
///
/// A permissive CORS layer is applied **only to the `/api/v1` surface** so the
/// local desktop/web UI (e.g. Vite on `127.0.0.1:5173`, or the Tauri shell)
/// can call the API cross-origin. This is safe because every API route still
/// requires the bearer token. The webui routes added afterwards (SPA, and the
/// unauthenticated loopback-only token bootstrap) are deliberately kept
/// same-origin: a page from any web origin must never be able to read the
/// bearer token.
///
/// With the `webui` feature, the embedded Desktop UI is served from `GET /`
/// (SPA with API JSON 404 fallbacks) and the loopback-only token bootstrap
/// endpoint is added.
pub fn router(state: Arc<AppState>) -> axum::Router {
    let cors = tower_http::cors::CorsLayer::permissive();
    // Api routes first: the CORS layer is applied to this router only.
    let api = Router::new()
        .route("/api/v1/status", get(status))
        .route("/api/v1/backends", get(backends))
        .route("/api/v1/events", get(events))
        .route("/api/v1/transfers", get(list_transfers).post(add_transfer))
        .route(
            "/api/v1/transfers/{id}",
            get(get_transfer).delete(delete_transfer),
        )
        .route("/api/v1/transfers/{id}/pause", post(pause_transfer))
        .route("/api/v1/transfers/{id}/resume", post(resume_transfer))
        .route("/api/v1/transfers/{id}/cancel", post(cancel_transfer))
        .route("/api/v1/transfers/{id}/files", get(transfer_files))
        .route("/api/v1/searches", get(list_searches).post(add_search))
        .route("/api/v1/searches/{id}", get(get_search))
        .route("/api/v1/searches/{id}/results", get(search_results))
        .route("/api/v1/searches/{id}/stop", post(stop_search))
        .route(
            "/api/v1/searches/{id}/results/{result_id}/download",
            post(download_result),
        )
        .route("/api/v1/settings", get(get_settings).put(put_settings))
        .route(
            "/api/v1/settings/{key}",
            get(get_setting).put(put_setting).delete(delete_setting),
        )
        .route("/api/v1/postprocess", get(list_postprocess))
        .route("/api/v1/postprocess/{id}", get(get_postprocess))
        .route("/api/v1/library", get(list_library))
        .merge(
            utoipa_swagger_ui::SwaggerUi::new("/api/v1/docs")
                .url("/api/v1/docs/openapi.json", ApiDoc::openapi()),
        )
        .layer(cors);

    // Webui routes are added AFTER `.layer(cors)` so the permissive CORS
    // (in axum, layers only wrap routes present when `.layer` is called) can
    // never expose the unauthenticated token bootstrap to cross-origin pages.
    #[cfg_attr(not(feature = "webui"), allow(unused_mut))]
    let mut app = api;
    #[cfg(feature = "webui")]
    {
        use axum::routing::get as webui_get;
        app = app
            .route("/", webui_get(crate::webui::index))
            .route("/__agpeer_token", webui_get(crate::webui::token))
            .fallback(crate::webui::spa_fallback);
    }

    app.with_state(state)
}

#[derive(utoipa::OpenApi)]
#[openapi(
    info(title = "agpeer API", version = "0.1.0"),
    components(schemas(
        StatusResponse,
        BackendStatus,
        MessageResponse,
        CancelRequest,
        SearchResponse
    ))
)]
pub struct ApiDoc;
