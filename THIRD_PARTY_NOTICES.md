# Third-Party Notices

agpeer builds on third-party software. This file records the dependencies,
their licenses, and — where the licensing imposes a component boundary — the
exact terms under which they are used.

This is a living document. **Contributing:** before adding a new dependency,
record it here with its name, purpose, license, and why existing dependencies
cannot cover it (see the dependency policy in `AGENTS.md`). Do not add a
dependency without updating this file.

---

## Embedded / linked dependencies (Rust workspace)

These crates are compiled into the agpeer core binary. Full license texts ship
with the crates in `Cargo.lock`; summaries and sources are listed here.

### rqbit / librqbit

- **Purpose:** BitTorrent engine, embedded behind the `TransferBackend` trait.
- **License:** Apache-2.0.
- **Source:** https://github.com/ikatson/rqbit

Used as an embedded library with its HTTP API disabled or bound to a private
loopback port. Apache-2.0 attribution requirements are preserved.

### sqlx

- **Purpose:** async SQLite access and migrations (source of truth for
  application state).
- **License:** Apache-2.0 OR MIT.
- **Source:** https://github.com/launchbadge/sqlx

### axum

- **Purpose:** HTTP framework for the versioned `/api/v1` API.
- **License:** MIT.
- **Source:** https://github.com/tokio-rs/axum

### tokio, tokio-stream, tokio-util

- **Purpose:** async runtime, streams (SSE), and utilities.
- **License:** MIT.
- **Source:** https://github.com/tokio-rs/tokio

### tower, tower-http

- **Purpose:** middleware (CORS, tracing) and service composition for the API.
- **License:** MIT.
- **Source:** https://github.com/tower-rs/tower , https://github.com/tower-rs/tower-http

### serde, serde_json

- **Purpose:** serialization of the typed API and storage models.
- **License:** Apache-2.0 OR MIT.
- **Source:** https://github.com/serde-rs/serde , https://github.com/serde-rs/json

### toml

- **Purpose:** static bootstrap configuration loading.
- **License:** Apache-2.0 OR MIT.
- **Source:** https://github.com/toml-rs/toml

### reqwest (if used)

- **Purpose:** HTTP client for the slskd sidecar API and remote `.torrent`
  URLs.
- **License:** Apache-2.0 OR MIT.
- **Source:** https://github.com/seanmonstar/reqwest

### Other workspace dependencies

The remaining direct dependencies (uuid, chrono, thiserror, async-trait, clap,
directories, keyring, utoipa, utoipa-swagger-ui, futures, tracing,
tracing-subscriber) are permissively licensed; their licenses are declared in
their `Cargo.toml` metadata and resolved via `Cargo.lock`.

---

## Managed sidecar (NOT linked into the core)

### rustsoseek

- **Purpose:** native Soulseek wire-protocol client (login, search, download,
  distributed search). Embedded behind the shared backend traits.
- **License:** MIT OR Apache-2.0.
- **Source:** https://github.com/Scarlet-Raine/RustSoSeek

`rustsoseek` is a clean-room implementation written from the public Soulseek
protocol documentation only; it contains no code translated or copied from
`slskd`, `Soulseek.NET`, `Nicotine+`, `aioslsk`, or `museek+`. It is linked as
a library dependency and its license texts are preserved.

---

## Desktop shell (apps/desktop)

### Tauri

- **Purpose:** desktop shell for the UI over the core API.
- **License:** MIT OR Apache-2.0.
- **Source:** https://github.com/tauri-apps/tauri

### React

- **Purpose:** UI framework for the desktop frontend.
- **License:** MIT.
- **Source:** https://github.com/facebook/react

### Vite

- **Purpose:** frontend build tooling.
- **License:** MIT.
- **Source:** https://github.com/vitejs/vite

---

## Third-party software used at runtime

### 7-Zip

- **Purpose:** archive extraction backend (`.zip`, `.rar`, `.7z`, `.tar`,
  `.tar.gz`, `.tgz`, multipart sets) invoked as an external process behind the
  `Extractor` adapter.
- **License:** GNU LGPL (unmodified binary, invoked as a subprocess).
- **Source:** https://www.7-zip.org/

The 7-Zip binary is invoked as an independent process; it is not linked into
the core. Any distribution of the 7-Zip binary follows its own license terms.

### ffprobe (optional)

- **Purpose:** media inspection behind an adapter. Absent ffprobe, inspection
  is skipped and never fails a job.
- **License:** GNU GPL-3.0 (external executable, invoked when available).
- **Source:** https://ffmpeg.org/

ffprobe is an optional external tool, never bundled, never linked.
