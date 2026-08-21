# HTTP API

The REST API is a product feature, versioned from day one under `/api/v1`.
It is deterministic, typed, inspectable, retry-safe where practical, and
explicit about destructive operations. Agents and the desktop UI are equal
clients of the same API.

OpenAPI documentation is generated from the Rust API definitions (via
`utoipa`) and served alongside the API.

## Base

- Base path: `http://127.0.0.1:41000/api/v1` (port configurable; loopback by default).
- Content type: `application/json`.
- Commands remain on REST; realtime events are SSE-only.

## Authentication

A bearer token is generated on first boot and is **required on all requests,
including loopback**.

```http
Authorization: Bearer <token>
```

- The token is persisted in the private data directory with restrictive file
  permissions; its location is printed at startup, its value is never logged.
- The Tauri shell reads the token automatically and injects it into every
  request. Agents read it from the same file.
- Requests without a valid token return `401` with error code
  `AuthenticationFailed`.

## Errors

Errors are typed and machine-readable:

```json
{
  "code": "TransferNotFound",
  "message": "transfer not found"
}
```

Stable codes include: `BackendUnavailable`, `AuthenticationFailed`,
`InvalidSource`, `SearchExpired`, `ResultExpired`, `TransferNotFound`,
`SearchNotFound`, `ResultNotFound`, `NotFound`, `PermissionDenied`,
`UnsafePath`, `ExtractionFailed`, `ProcessLaunchDenied`,
`SidecarVersionUnsupported`, `InvalidState`, `Database`, `Backend`,
`Internal`. Internal detail and secrets are never returned over the API; they
belong in local logs only.

## The normalized Transfer

All backends produce the same shape:

```json
{
  "id": "3f2a…uuid…",
  "backend": "torrent",
  "source": "magnet:?xt=urn:btih:…",
  "display_name": "example.torrent",
  "state": "downloading",
  "progress": 0.42,
  "bytes_total": 104857600,
  "bytes_completed": 44040192,
  "download_rate": 1048576,
  "upload_rate": 65536,
  "eta": 58,
  "destination": "D:\\Downloads",
  "created_at": "2026-08-15T00:00:00Z",
  "started_at": "2026-08-15T00:00:01Z",
  "completed_at": null,
  "error": null,
  "files": [
    {
      "index": "0",
      "path": "example/file.bin",
      "size": 104857600,
      "selected": true,
      "bytes_completed": 44040192
    }
  ],
  "postprocess_state": "pending",
  "metadata": {}
}
```

`backend` is `torrent`, `soulseek`, or `hook` (search only). `state` is one of: `queued`,
`resolving`, `downloading`, `paused`, `verifying`, `completed`,
`postprocessing`, `ready`, `failed`, `cancelled`, `orphaned`. Backend-specific
fields live in the namespaced `metadata` map, never in the common schema.
See [job-model.md](job-model.md) for the full model.

## Endpoints

### Status and backends

| Method | Path | Description |
|---|---|---|
| `GET` | `/status` | Core health: version, uptime, database ok, per-backend state. |
| `GET` | `/backends` | Registered backends and their runtime state (`ready` / `degraded` / `unavailable`). |

`GET /api/v1/status` response sketch:

```json
{
  "version": "0.1.0",
  "uptime_seconds": 120,
  "database": "ok",
  "backends": {
    "torrent": "ready",
    "soulseek": "degraded"
  }
}
```

### Transfers

| Method | Path | Description |
|---|---|---|
| `POST` | `/transfers` | Add a transfer (magnet, `.torrent` path, remote `.torrent` URL, or a `soulseek:` result id). |
| `GET` | `/transfers` | List transfers; optional `?state=` / `?backend=` filters. |
| `GET` | `/transfers/{id}` | Fetch one transfer. |
| `GET` | `/transfers/{id}/files` | List files with selection status (used for pre-download selection). |
| `POST` | `/transfers/{id}/pause` | Pause. |
| `POST` | `/transfers/{id}/resume` | Resume. |
| `POST` | `/transfers/{id}/cancel` | Cancel the transfer. |
| `DELETE` | `/transfers/{id}` | Remove the job; `?delete_data=true` deletes downloaded files (default `false`). |

`POST /api/v1/transfers` request:

```json
{
  "backend": "torrent",
  "source": "magnet:?xt=urn:btih:…",
  "destination": "D:\\Downloads",
  "display_name": "optional",
  "file_selection": [{"index": "0", "selected": true}],
  "metadata": {}
}
```

Response: `201 Created` with `{"transfer_id": "<uuid>"}`.

Destructive actions (`cancel`, `DELETE`) are explicit: they require the
corresponding HTTP method and never delete data unless `delete_data` is set.

### Searches

| Method | Path | Description |
|---|---|---|
| `POST` | `/searches` | Start a search. |
| `GET` | `/searches` | List searches. |
| `GET` | `/searches/{id}` | Fetch search status (state, result count, expiry). |
| `GET` | `/searches/{id}/results` | Fetch accumulated results. |
| `POST` | `/searches/{id}/stop` | Stop the search. |
| `POST` | `/searches/{id}/results/{result_id}/download` | Download a result. |

`POST /api/v1/searches` request:

```json
{
  "backend": "soulseek",
  "query": "artist album flac",
  "user": "optional-user",
  "extension": "flac",
  "min_size": 1000000,
  "max_results": 1000
}
```

Response: `201 Created` with `{"search_id": "<uuid>"}`. New results arrive as
`search.result` SSE events and are retrievable via
`GET /searches/{id}/results`.

`POST /api/v1/searches/{search_id}/results/{result_id}/download`:

```json
{
  "destination": "D:\\Downloads"
}
```

Result IDs are application-generated opaque IDs, so an agent never has to
reconstruct a raw Soulseek transfer request. Results expire per the search
TTL (default 24 h); expired results answer `ResultExpired` / `SearchExpired`.

### Post-processing

| Method | Path | Description |
|---|---|---|
| `POST` | `/postprocess` | Create a post-processing job for a transfer. |
| `GET` | `/postprocess` | List jobs; optional `?transfer_id=` filter. |
| `GET` | `/postprocess/{id}` | Fetch a job with its step states. |

`POST /api/v1/postprocess` request:

```json
{
  "transfer_id": "<uuid>",
  "target": "0",
  "steps": ["extract", "organize"],
  "confirmation_token": "optional-token-for-run_installer"
}
```

- `target` is the file-scoped target (transfer file index/path); one transfer
  spawns 0..n jobs.
- If `steps` is omitted, the rule-driven default pipeline applies.
- `confirmation_token` is required when the plan includes the privileged
  `run_installer` step (see [postprocessing.md](postprocessing.md)).
- Every step is individually observable and retryable; failed steps surface
  typed errors per step.

### Events (SSE)

| Method | Path | Description |
|---|---|---|
| `GET` | `/events` | Server-Sent Events stream (requires bearer token). |

Event kinds:

```text
backend.ready            backend.degraded
search.started           search.result        search.completed   search.failed
transfer.added           transfer.started     transfer.progress  transfer.paused
transfer.completed       transfer.failed      transfer.removed
postprocess.started      postprocess.step_started   postprocess.step_completed
postprocess.completed    postprocess.failed
```

Each event is one SSE `data:` frame carrying JSON, e.g.
`{"kind":"transfer.progress","payload":{…}}`. Progress events are throttled
per stream.

### Settings

| Method | Path | Description |
|---|---|---|
| `GET` | `/settings` | List runtime settings. |
| `GET` | `/settings/{key}` | Fetch one setting. |
| `PUT` | `/settings/{key}` | Set a runtime setting. |

Settings are persisted in the SQLite `settings` table. Static bootstrap
values (ports, paths) are configured in the TOML file, not
here. Secrets are never settable or readable through this API.

`hook_search.enabled` (boolean) controls whether the magnet-search hook is
permitted to run searches; it is seeded from `[hook_search].enabled` on first
boot and toggled at runtime (e.g. from the desktop Settings page). New
`hook` searches return `503 BackendUnavailable` while it is `false`.

`hook_search.domains` (JSON array of strings) is the list of sites/indexes the
hook command should query. It is seeded from `[hook_search].domains`, edited at
runtime in Settings, and passed to the hook command at search time: a
`{domains}` token in the command is replaced with the comma-joined list,
otherwise the list is appended as trailing arguments (after the query).

## Command-style examples

The API is designed to be mapped one-to-one to agent tool calls:

```text
torrent_add_magnet(uri, destination?)      → POST /api/v1/transfers
torrent_add_file(file, destination?)       → POST /api/v1/transfers
soulseek_search(query, filters?)           → POST /api/v1/searches
soulseek_get_results(search_id)            → GET  /api/v1/searches/{id}/results
soulseek_download(result_id, destination?) → POST /api/v1/searches/{sid}/results/{rid}/download
transfer_list(filter?)                     → GET  /api/v1/transfers
transfer_get(id)                           → GET  /api/v1/transfers/{id}
transfer_pause(id)                         → POST /api/v1/transfers/{id}/pause
transfer_resume(id)                        → POST /api/v1/transfers/{id}/resume
transfer_cancel(id, delete_data=false)     → POST /api/v1/transfers/{id}/cancel
postprocess_extract(id)                    → POST /api/v1/postprocess
postprocess_organize(id, library)          → POST /api/v1/postprocess
postprocess_run_installer(id, confirmation_token) → POST /api/v1/postprocess
```

Unrestricted shell execution is **not** exposed as an agent tool. The future
MCP adapter is a thin layer over this API with no separate business logic.
