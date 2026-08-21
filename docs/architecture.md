# Architecture

This document describes the runtime architecture of agpeer. It is the design
reference for contributors; `AGENTS.md` remains the authoritative spec.

## Components

The workspace is split into library crates plus a desktop shell. All
protocol-specific behavior stays behind adapters.

| Component | Responsibility |
|---|---|
| `crates/common` | Shared typed models (`Transfer`, `TransferState`, `SearchResult`, …), opaque IDs (`TransferId`, `SearchId`, `ResultId`), the typed `Error` enum, and the backend abstraction traits. Single source of truth for the normalized model. |
| `crates/storage` | SQLite persistence via `sqlx`: migrations plus stores for transfers, transfer files, searches, search results, post-processing jobs and steps, settings, events/audit, and backend state. |
| `crates/core` | Application state, TOML bootstrap config loading, the secret-store abstraction, and the event bus (tokio broadcast → SSE fan-out with per-stream throttling). |
| `crates/api` | Axum HTTP server: versioned `/api/v1` routes, bearer-token middleware, SSE endpoint, OpenAPI generation (`utoipa`). |
| `crates/torrent` | `TransferBackend` implementation over embedded `librqbit`. Normalizes rqbit state into `TransferState`; backend metadata lives in a namespaced `metadata` field. |
| `crates/soulseek` | Thin adapter mapping the `rustsoseek` native Soulseek client (`SearchBackend` + `TransferBackend`) into the shared model. Wire-format types stay in `rustsoseek`. |
| `crates/jobs` | Post-processing job and step model (`Job`, `Step`, `StepKind`, states). |
| `crates/postprocess` | Post-processing engine: classifier, `Extractor` adapter, media organization, installer policy (Phase 3). |
| `apps/desktop` | Tauri 2 + React + TypeScript shell. Spawns and manages the core process, reads/injects the bearer token, and renders the UI over the core API only. |

## Process model

- A single core binary, `agpeer`, provides `serve` and `migrate` subcommands.
  The root package builds this binary.
- `agpeer migrate` applies SQLite migrations; `agpeer serve` starts the Axum
  API, initializes backends, and reconciles state.
- The Tauri desktop shell spawns the core as a child process and health-polls
  it until ready. The core also runs standalone, so the REST API is usable
  without the desktop UI.
- The Soulseek wire protocol is handled directly by the `rustsoseek` library
  (login, search, download, distributed search) over TCP to the Soulseek
  server; no sidecar process is involved.

## Configuration split

- **TOML** is static bootstrap configuration: ports, paths, credential
  references. Loaded by `crates/core`.
- **SQLite `settings` table** holds runtime-settable settings exposed via
  `/api/v1/settings`. The TOML file is not rewritten at runtime.

## Backend abstraction

Every transfer and search backend normalizes into traits in `crates/common`.
The rest of the application never touches provider-specific types.

```rust
#[async_trait]
pub trait TransferBackend: Send + Sync {
    fn backend(&self) -> Backend;
    async fn add(&self, request: AddTransferRequest) -> Result<Transfer>;
    async fn get(&self, id: &TransferId) -> Result<Transfer>;
    async fn list(&self) -> Result<Vec<Transfer>>;
    async fn pause(&self, id: &TransferId) -> Result<()>;
    async fn resume(&self, id: &TransferId) -> Result<()>;
    async fn cancel(&self, id: &TransferId, delete_data: bool) -> Result<()>;
    async fn forget(&self, id: &TransferId) -> Result<()> { ... }
}

#[async_trait]
pub trait SearchBackend: Send + Sync {
    fn backend(&self) -> Backend;
    async fn search(&self, request: SearchRequest) -> Result<SearchId>;
    async fn results(&self, id: &SearchId) -> Result<Vec<SearchResult>>;
    async fn stop(&self, id: &SearchId) -> Result<()>;
}
```

For v1:

- Torrent implements `TransferBackend`.
- Soulseek implements both `SearchBackend` and `TransferBackend`.
- Hook (`[hook_search]`) implements `SearchBackend` only: a user-configured
  external command returns magnet links, which are pulled through the torrent
  backend. It never owns transfers. The runtime `hook_search.domains` setting
  (editable in Settings) is handed to the command at search time so it knows
  which sites/indexes to query.
- HTTP/direct downloads may implement `TransferBackend` later.

IDs are opaque, application-owned UUIDs; backend IDs are never exposed as the
canonical identity of a transfer or search result.

## Event bus and SSE

`crates/core` hosts an in-memory event bus (tokio broadcast). `crates/api`
fans events out to connected SSE clients on `GET /api/v1/events` with
per-stream throttling so progress events cannot flood UI or agents.

Event kinds (see `AGENTS.md`):

```text
backend.ready            backend.degraded
search.started           search.result        search.completed   search.failed
transfer.added           transfer.started     transfer.progress  transfer.paused
transfer.completed       transfer.failed      transfer.removed
postprocess.started      postprocess.step_started   postprocess.step_completed
postprocess.completed    postprocess.failed
```

Progress events are throttled.

## Persistence and reconciliation

SQLite is the source of truth for application-owned state. Tables:
`transfers`, `transfer_files`, `searches`, `search_results`, `postprocess_jobs`,
`postprocess_steps`, `settings`, `events`, `backend_state`.

Reconciliation rules:

- **Backend is authoritative.** Never assume a backend still contains a
  transfer simply because the database says it does.
- **Orphaned terminal state.** A transfer present in the database but missing
  from its backend at startup is marked `orphaned` (a terminal state). Files
  are **never auto-deleted**.
- **Import backend-only transfers.** Transfers present in the backend but not
  in the database are imported on reconciliation.
- **Reconcile on startup and after backend recovery.** The Soulseek backend
  restart path marks the backend degraded, preserves job records, captures
  logs, restarts with bounded exponential backoff, then reconciles active
  transfers and emits recovery events.

Search results are bounded and expiring (default TTL 24 h); transfer records
persist.
