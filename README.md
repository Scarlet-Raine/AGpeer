# agpeer

A polished, automation-first P2P download manager for humans and software agents.

agpeer is a general-purpose transfer and post-processing client. It handles
user-supplied BitTorrent magnets / `.torrent` files and Soulseek
search/downloads under one normalized job model, exposes the same capabilities
through a stable local REST API, and provides a desktop UI over that API.

The project is intentionally provider-neutral on the torrent side. It does not
build or bundle torrent indexers, warez catalogues, DRM bypasses, cracks, key
generators, or first-party discovery integrations for unauthorized commercial
content.

## Features

- **BitTorrent** — add magnets, local `.torrent` files, and remote `.torrent`
  URLs; per-file selection; pause/resume/cancel; progress, peers, speeds, ETA;
  restart/resume persistence.
- **Soulseek** — search with incremental results, file and folder
  downloads, queue position and progress — via the native `rustsoseek` client.
- **One normalized job model** — transfers from every backend normalize into a
  single typed model with opaque application-owned IDs.
- **Post-processing** — rule-driven pipeline (classify, verify, extract,
  inspect, organize) with safe archive extraction and explicit installer
  execution policy.
- **Agent-first REST API** — versioned `/api/v1`, bearer-token auth, SSE
  events, OpenAPI documentation. The core service is the product; the desktop
  UI, CLI, and future MCP adapter are clients of it.
- **Local-first and private** — loopback binding by default, secrets in OS
  secure storage, no silent firewall changes.

## Architecture

Rust-first architecture:

| Layer | Technology |
|---|---|
| Core / service | Rust (`crates/`), Tokio |
| Torrent engine | `librqbit` embedded behind a `TransferBackend` trait |
| Soulseek engine | `rustsoseek` native client, clean-room wire protocol |
| HTTP API | Axum, versioned `/api/v1`, bearer token required |
| Realtime events | Server-Sent Events (`/api/v1/events`) |
| Persistence | SQLite (`sqlx`), source of truth for application state |
| Desktop shell | Tauri 2 + React + TypeScript + Vite |
| Agent interface | REST first (`/api/v1`); MCP server (`agpeer-mcp`) over it |

The core service runs as a single binary (`agpeer`). The Tauri shell spawns it
as a child process; the core also runs standalone for API-only use. See
[docs/architecture.md](docs/architecture.md) for details.

## Repository layout

```text
/
├─ crates/
│  ├─ common/       shared typed models, opaque IDs, errors, backend traits
│  ├─ storage/      SQLite (sqlx) migrations and stores
│  ├─ core/         app state, config, secrets, event bus
│  ├─ api/          Axum HTTP API + SSE + bearer-token auth
│  ├─ torrent/      librqbit-backed TransferBackend
│  ├─ soulseek/     rustsoseek adapter (SearchBackend + TransferBackend)
│  ├─ jobs/         post-processing job/step model
│  ├─ postprocess/  post-processing engine (Phase 3)
│  ├─ mcp/          MCP server bridging agents to the REST API
│  └─ debug-mcp/    MCP server for fast, low-token debugging (logs/code/git)
├─ apps/desktop/    Tauri 2 + React + TypeScript shell
├─ docs/            architecture, API, job model, post-processing, security
└─ tests/           integration tests and fixtures
```

## Quickstart

### Prerequisites

- Rust stable (1.80+, per the workspace `rust-version`)
- Node.js 20+ (for the desktop shell)
- 7-Zip (`7z.exe` / `7zz`) on `PATH` for archive extraction

### Build and run the core

The default build includes the real `librqbit` torrent engine (the `rqbit`
feature is enabled by default):

```bash
cargo build --release --bin agpeer
# binary: target/release/agpeer.exe
```

The repository ships a ready-made runtime environment under `run/`:

```bash
# Edit run/config.toml and fill in your Soulseek username/password first.
powershell -ExecutionPolicy Bypass -File .\run\start.ps1
```

`start.ps1` builds the binary if needed, runs migrations, then starts the core
on `127.0.0.1:41000` with:
- the **rqbit** torrent engine (magnet / `.torrent` / HTTPS adds),
- the native **rustsoseek** Soulseek client (login, search, download over the
  wire protocol),
- automatic **post-processing**: completed downloads are moved into
  `E:\Media` (configurable via `[postprocess].library_root`) as a
  Jellyfin/Plex-friendly tree
  (`<root>/TV Shows/<Title>/Season NN/`, `<root>/Movies/<Title> (<Year>)/`,
  `<root>/Music/<Artist>/<Album>/`, …).

The core connects directly to the Soulseek server using the `username` /
`password` in `run/config.toml`; no sidecar process or external engine is
required.

For API-only/manual workflow:

```bash
agpeer migrate --config run/config.toml
agpeer serve   --config run/config.toml
```

By default the core listens on `127.0.0.1:41000`. On first boot it generates a
bearer token, persists it in the private data directory, and prints the
location — the token value itself is never written to logs. The Tauri shell
reads the token automatically; agents can read it from the same file.

### Run the desktop app

```bash
cd apps/desktop
npm ci
npm run tauri dev
```

The desktop UI talks only to the core API — it never touches backends or the
database directly.

## The API is the product

The core service is the product. The desktop UI, CLI, MCP server, and future
integrations are clients of it. REST commands work without the desktop UI
running, as long as the core service is running:

```bash
# Add a magnet
curl -X POST http://127.0.0.1:41000/api/v1/transfers \
  -H "Authorization: Bearer $AGPEER_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"backend":"torrent","source":"magnet:?xt=urn:btih:..."}'

# List transfers
curl http://127.0.0.1:41000/api/v1/transfers \
  -H "Authorization: Bearer $AGPEER_TOKEN"
```

See [docs/api.md](docs/api.md) for the full endpoint reference and
[docs/job-model.md](docs/job-model.md) for the normalized transfer model.

### For coding agents (MCP)

`agpeer-mcp` exposes the same /api/v1 surface as MCP tools over stdio, so
Claude Code, Kilo, Cursor, and other MCP clients can drive downloads and
Soulseek searches. See [docs/mcp.md](docs/mcp.md) for build, run, and
`mcpServers` configuration examples. For fast, low-token debugging of logs,
source, and git state, `agpeer-debug-mcp` is a companion MCP server — see
[docs/mcp-debug.md](docs/mcp-debug.md).

## Security posture

- Core API binds to `127.0.0.1` by default; LAN binding is an explicit,
  advanced-user action that requires authentication.
- A bearer token is required on **all** requests, including loopback.
- Secrets live in OS secure storage (Windows DPAPI primary, keyring fallback);
  they are never committed, logged, or returned over the API.
- Download roots are enforced and canonicalized; archive contents are treated
  as hostile (path-traversal protected).
- Downloaded executables are never auto-launched. `run_installer` is an
  explicit, confirmed, audited action.

See [docs/security.md](docs/security.md) for the full model.

## Documentation

- [docs/architecture.md](docs/architecture.md) — components, process model, reconciliation
- [docs/api.md](docs/api.md) — REST API reference
- [docs/job-model.md](docs/job-model.md) — normalized transfer model
- [docs/postprocessing.md](docs/postprocessing.md) — post-processing pipeline
- [docs/security.md](docs/security.md) — security model
- [docs/mcp.md](docs/mcp.md) — MCP server for coding agents
- [docs/mcp-debug.md](docs/mcp-debug.md) — debugging MCP server (logs/code/git)
- [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) — third-party licensing
- [tests/README.md](tests/README.md) — testing strategy

## License

Dual-licensed under [MIT OR Apache-2.0](LICENSE). See
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for the full third-party
licensing record. The Soulseek wire-protocol client is `rustsoseek`
([MIT OR Apache-2.0](https://github.com/Scarlet-Raine/RustSoSeek)).
