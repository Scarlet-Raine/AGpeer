# AGENTS.md

## Purpose

This is the operating guide for coding agents working on agpeer. Keep it short,
current, and focused on decisions that affect implementation. Product detail
belongs in `docs/`; code, manifests, migrations, and tests describe the current
implementation.

agpeer is a local-first, automation-first P2P transfer and post-processing
client. It supports caller-supplied BitTorrent sources and Soulseek
search/downloads under one normalized job model, exposed through a stable local
API and a Tauri desktop client.

The torrent side is provider-neutral. Do not add torrent indexers, content
catalogues, scraping integrations, DRM bypasses, cracks, key generators, binary
patching, or features designed to evade network rules.

## Start here

Before changing code:

1. Read this file once.
2. Read the task-relevant document and crate; do not load every document.
3. Search for the existing type, route, adapter, setting, or test before adding
   an abstraction.
4. Check the working tree when Git metadata is available and preserve unrelated
   user changes.
5. State assumptions only when they materially affect behavior or scope.

Use this routing table:

| Work area | Read first | Primary code |
|---|---|---|
| Architecture/boundaries | `docs/architecture.md` | `crates/core`, `src/main.rs` |
| Shared models/errors | `docs/job-model.md` | `crates/common` |
| REST, auth, SSE, OpenAPI | `docs/api.md` | `crates/api` |
| Torrent behavior | `crates/torrent/SPIKE.md` when relevant | `crates/torrent` |
| Soulseek | `fresh-workspace/SOULSEEK_REWRITE.md` | `crates/soulseek`, `rustsoseek` |
| Post-processing/file safety | `docs/postprocessing.md`, `docs/security.md` | `crates/postprocess`, `crates/jobs` |
| Persistence/reconciliation | `docs/architecture.md`, migrations | `crates/storage`, `crates/core` |
| Desktop UI | `README.md`, API types | `apps/desktop/src`, `apps/desktop/src-tauri` |
| Agent MCP | `docs/mcp.md` | `crates/mcp` |
| Debug MCP | `docs/mcp-debug.md` | `crates/debug-mcp` |
| Tests/fixtures | `tests/README.md` | in-crate tests, `tests/` |

When prose and implementation disagree, do not guess. Preserve the hard
constraints here, inspect the relevant code/tests, and update stale docs in the
same change. Public API behavior is not changed solely by editing prose.

## Current architecture

This is an implemented Rust workspace, not a phase-zero skeleton:

- `agpeer` is the core binary with `serve` and `migrate` commands.
- `crates/common` owns normalized models, opaque IDs, typed errors, and backend
  traits.
- `crates/storage` owns SQLite migrations and stores.
- `crates/core` owns state, config, secrets, events, reconciliation,
  housekeeping, and post-processing coordination.
- `crates/api` owns the Axum `/api/v1` API, auth, SSE, DTOs, and OpenAPI.
- `crates/torrent` embeds `librqbit` behind the transfer abstraction.
- `crates/soulseek` is a thin adapter that maps the `rustsoseek` native client
  into the shared backend traits.
- `rustsoseek` is a separate clean-room native Soulseek wire-protocol client
  (login, search, download, distributed search), maintained at
  https://github.com/Scarlet-Raine/RustSoSeek.
- `crates/jobs` and `crates/postprocess` own observable post-processing jobs.
- `crates/mcp` is a thin MCP-to-REST client for operating agpeer.
- `crates/debug-mcp` is a bounded, read-only source/log/Git inspection MCP.
- `apps/desktop` is a Tauri 2 + React/TypeScript client of the REST API.

The core service is the product. Desktop and MCP are clients; they must not
bypass the API/core to implement separate business logic.

## Non-negotiable boundaries

### Backends and shared models

- Keep protocol-specific behavior behind adapters.
- Torrent implements `TransferBackend`; do not force it into `SearchBackend`.
- Soulseek implements both traits through the `rustsoseek` native client.
- Use application-owned opaque IDs. Backend IDs and metadata remain namespaced.
- Shared model changes start in `crates/common`, then propagate through
  storage, API DTOs/OpenAPI, MCP, UI types, tests, and docs as applicable.
- Do not leak rqbit or rustsoseek wire types outside their adapter crate.

### rustsoseek licensing boundary

`rustsoseek` is a clean-room implementation of the Soulseek wire protocol,
written from the public protocol documentation only (`SLSKPROTOCOL.md`, Museek+
wiki). It is Apache-2.0 and is maintained in its own repository. This is an
architecture and licensing constraint:

- Do not copy `slskd`, `Soulseek.NET`, `aioslsk`, or `Nicotine+` source into
  agpeer or rustsoseek.
- Keep every wire-format type, message parser, and handshake routine inside
  `rustsoseek`; do not leak them into `crates/common` or the UI.
- Preserve the clean-room provenance note in `rustsoseek`'s README and in
  `THIRD_PARTY_NOTICES.md`.

`librqbit` is Apache-2.0; preserve its license and attribution requirements.

### API and persistence

- Version public routes under `/api/v1`; keep responses typed and
  machine-readable.
- Require bearer authentication on every request, including loopback.
- Commands use REST; realtime notifications use SSE. Throttle progress events.
- SQLite is the source of truth for application-owned state, while backends are
  authoritative for live transfer state. Reconcile at startup and after
  recovery; never delete files merely because a backend lost a job.
- Search results may expire; transfer and audit records persist.
- Never return stack traces, secrets, auth headers, or backend-private JSON.
- MCP maps one-to-one or nearly one-to-one to safe REST operations. Do not add
  separate business logic or unrestricted shell execution to MCP.

### Security and filesystem

- Bind core to loopback by default. LAN exposure and firewall changes require
  explicit user configuration; authentication remains required.
- Never commit or log credentials, tokens, cookies, keys, auth headers, runtime
  databases, logs, or downloaded content.
- Treat `run/`, downloaded payloads, and backend runtime directories as private
  data. Inspect them only for an explicit runtime-diagnosis task, and redact
  secrets and personal filenames from output.
- Canonicalize user paths, enforce configured roots, reject `..` traversal, and
  defend against archive traversal and symlink escapes.
- Do not silently move, overwrite, or delete files outside configured roots.
  Preserve source archives until extraction and output validation succeed.
- Downloaded executables are untrusted. Extraction never implies execution.
  Launch must be explicit, authorized as configured, and audited with origin.
- Sharing is off or empty until explicitly configured by the user.

## How to make changes

For review, explanation, diagnosis, or planning requests, inspect and report;
do not edit unless the user asks for a change. For build, fix, or refactor
requests, make the smallest coherent in-scope change and run relevant
non-destructive checks without asking first.

For implementation work:

1. Trace the current path end to end before editing.
2. Change the owning layer; avoid shims in unrelated layers.
3. Add/update tests with the behavior, including failure and authorization
   paths where relevant.
4. Update API/UI/MCP types and docs when a public contract changes.
5. Format touched code and run the narrowest useful checks first.
6. Expand to workspace checks for cross-crate or shared-type changes.
7. Report changes, validation, remaining risk, and skipped checks. Never call a
   placeholder or untested live integration complete.

Safe local reads, in-scope edits, formatting, builds, and tests are expected.
Ask before destructive actions, external writes, public-network/live Soulseek
tests, executable launch, or a material expansion of scope.

### Dependencies

Confirm existing dependencies cannot cover the need. Prefer maintained,
narrow, permissively licensed libraries for linked code. Record purpose and
license, update lockfiles, and treat GPL/AGPL linkage changes as architecture
and licensing decisions.

## MCP and tool use

Use available MCPs when they reduce uncertainty or avoid large unstructured
outputs. Availability varies by client, so workflows must still work with repo
search and normal build/test commands.

- `agpeer` MCP operates the running app through `/api/v1`: status, transfers,
  searches, and post-processing. It does not edit source.
- `agpeer-debug` is read-only and returns bounded source, log, and Git snippets.
- MCP output is runtime evidence, not a substitute for owning code, tests,
  migrations, or API docs.
- Verify the core is running before treating an MCP connection failure as a
  product bug. See `docs/mcp.md` and `docs/mcp-debug.md`.
- Never paste or return the bearer token. Prefer token-file/data-dir references.
- For uncertain third-party APIs, use an authoritative documentation MCP when
  available; otherwise use official upstream docs. Do not guess unstable APIs.

Use `rg`/`rg --files` for discovery. Exclude `target`, `node_modules`, `dist`,
and `run` unless explicitly in scope.

## Validation matrix

Choose checks in proportion to the change. Documentation-only changes need
reference, link, and command review—not a full rebuild.

### Rust

Narrow checks during iteration:

```powershell
cargo fmt -p <package> -- --check
cargo clippy -p <package> --all-targets --all-features -- -D warnings
cargo test -p <package>
```

Cross-cutting or final workspace checks:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

The full workspace includes `rmcp` through `agpeer-mcp`, which may require a
newer Rust toolchain than the root package's declared MSRV. Use the declared
MSRV for packages that promise it and the resolved workspace requirement for
workspace-wide MCP checks; report the distinction.

### Desktop

Run from `apps/desktop`:

```powershell
npm ci
npm run typecheck
npm run build
```

Run `npm run lint` only when ESLint and its configuration are present; the
script alone does not make lint a working gate. Do not edit generated `dist/`.

### Integration behavior

- Keep normal CI offline and deterministic.
- Torrent tests use legal local fixtures or locally generated torrents.
- Soulseek tests use mocks; public-network tests are opt-in only.
- Archive tests cover traversal, symlinks, partial extraction, and source
  preservation on failure.
- API changes test auth, typed errors, destructive defaults, and OpenAPI/DTO
  consistency.
- A vertical feature works through the core API; UI/MCP consume that capability
  where applicable.

## Source control and generated/runtime files

- Preserve unrelated changes; keep patches focused; do not rewrite history.
- Do not mass-format untouched code. Commit lockfiles with dependency changes.
- Never commit `.env*`, tokens, credentials, downloads, runtime SQLite files,
  logs, or extraction scratch space.
- Do not edit `target/`, frontend `dist/`, or generated Tauri schemas unless
  the task explicitly requires regeneration.
- If Git metadata is unavailable, say so instead of claiming a clean tree or a
  diff you could not inspect.

## Definition of done

A change is done when all applicable statements are true:

- It lives in the owning layer with backend boundaries intact.
- It is reachable through the core API; clients do not bypass core.
- Restart persistence/reconciliation is correct where applicable.
- Errors are typed, secrets redacted, and destructive/executable actions remain
  explicitly authorized.
- Relevant tests pass; skipped checks and environment blockers are reported.
- Affected API, MCP, UI types, config examples, notices, and docs are updated.
- No runtime data, secrets, unrelated edits, or generated artifacts entered the
  patch.

## Out of scope for v1

Do not spend time on torrent discovery/index providers, site scraping, DRM
circumvention, crack/keygen or binary patching workflows, unrestricted agent
shell access, multi-node orchestration, transcoding, a custom media player,
browser extensions, or mobile apps. The Soulseek wire protocol itself is owned
by `rustsoseek`; grow it there, in its own repository, and pull it in as a
tagged dependency. Prioritize reliable transfers, search, API behavior,
post-processing, recovery, security, and UX.
