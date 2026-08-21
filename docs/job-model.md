# Job Model

All transfer backends normalize into one internal model. This document defines
that model, its states, identifiers, and lifecycle rules.

## Unified transfer model

A transfer contains, at minimum:

```text
id                opaque application-owned ID (UUID)
backend           torrent | soulseek
source            original source supplied by the caller (magnet URI, path, URL, result id)
display_name      human-readable name
state             normalized state (below)
progress          0.0 ..= 1.0
bytes_total       Option<u64>
bytes_completed   u64
download_rate     Option<u64> bytes/sec
upload_rate       Option<u64> bytes/sec
eta               Option<u64> estimated seconds remaining
destination       destination directory
created_at        RFC3339 UTC
started_at        Option<RFC3339 UTC>
completed_at      Option<RFC3339 UTC>
error             Option<String>
files[]           per-file entries
postprocess_state none | pending | running | completed | failed
metadata          backend-specific, namespaced (never in the common schema)
```

`files[]` entries carry a backend-provided `index` (used as a stable
reference for selection and post-processing targets), a `path`, `size`,
`selected`, and `bytes_completed`.

## Normalized states

```text
queued          accepted, waiting to start
resolving       metadata/source resolution in progress (e.g. magnet metadata)
downloading     actively transferring
paused          paused by the user or backend
verifying       verifying downloaded data
completed       download finished; transfer data is on disk
postprocessing  post-processing jobs are running
ready           post-processing finished; usable
failed          failed; `error` describes why
cancelled       cancelled by the user
orphaned        exists in the database but was missing from the backend at
                startup reconciliation (terminal; files never auto-deleted)
```

Terminal states: `ready`, `failed`, `cancelled`, `orphaned`. A transfer can
reach `ready` either directly from `completed` (no post-processing) or after
post-processing completes.

## Backends

- `torrent` — embedded `librqbit` behind the `TransferBackend` trait. Sources:
  magnet URIs, local `.torrent` paths, and remote `.torrent` URLs. Supports
  per-file selection, pause/resume/cancel, rate limits, configurable listen
  port, and private-torrent handling. No torrent index/search is built into
  the core; sources are always supplied by the caller.
- `soulseek` — native `rustsoseek` client behind both `SearchBackend` and
  `TransferBackend`. Sources: search results (expressed as `soulseek:`
  result ids) or explicit user/path sources. All wire-format types stay in
  `rustsoseek` (see https://github.com/Scarlet-Raine/RustSoSeek).

## Opaque IDs

Backend-specific identifiers are never the canonical identity of any object.
Every object an agent (or the UI) can act on receives a random
application-owned UUID:

- `TransferId` — a normalized transfer.
- `SearchId` — a search.
- `ResultId` — a single search result.

Agents interact with these opaque IDs only, e.g.
`POST /api/v1/searches/{search_id}/results/{result_id}/download`. They never
need to reconstruct a raw Soulseek or rqbit identifier.

## Search results and expiry

- Searches and their results persist in SQLite (`searches`,
  `search_results`).
- Result sets are bounded (`max_results`, default 1000) and expiring.
- Default search-result TTL is 24 h. On expiry, results are evicted and the
  corresponding search is stopped.
- Expired searches/results answer typed errors (`SearchExpired`,
  `ResultExpired`).
- Transfer records, by contrast, persist indefinitely.

## Post-processing job granularity

- A `postprocess_job` belongs to exactly one transfer and targets a specific
  file within it (`target` = transfer file index/path).
- One transfer spawns **0..n jobs**, each with its own state, ordered steps,
  and error.
- Each job runs an ordered step list where every step is individually
  observable and retryable:

```text
verify
extract
flatten
rename
inspect_media
move
copy
hardlink
cleanup
run_installer
custom_hook
```

- Job states: `pending`, `running`, `completed`, `failed`, `cancelled`.
- Step states: `pending`, `running`, `completed`, `failed`, `skipped`.

If post-processing is not applicable to a transfer, `postprocess_state` is
`none` and no jobs are created.

## Reconciliation

SQLite is the source of truth for application-owned state; the backend is
authoritative about what it actually holds:

- backend-only transfers are imported at reconciliation;
- database-only transfers missing from the backend become `orphaned`
  (terminal) — files are never auto-deleted;
- reconciliation runs at startup and after backend recovery.
