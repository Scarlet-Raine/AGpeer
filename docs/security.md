# Security Model

agpeer is local-first. The security posture is: private by default,
explicit for everything that widens exposure, hard boundaries around secrets
and executable content.

## Network

- **Loopback by default.** The core API binds to `127.0.0.1:41000`
  (configurable).
- **LAN binding is an explicit, advanced-user action**, requires explicit
  configuration, and requires authentication on the listener.
- **No silent firewall changes.** The application never opens firewall rules
  without the user's explicit action.
- Embedded rqbit's HTTP API (if enabled) binds to a private loopback port only.

## Service / headless deployment

- Bind host, port, data dir, and Soulseek credentials are overridable via
  environment variables (`AGPEER_HOST`, `AGPEER_PORT`, `AGPEER_DATA_DIR`,
  `AGPEER_SOULSEEK_*`, `AGPEER_CONFIG`), so services (NSSM on Windows,
  systemd/Docker on Linux) run without editing a TOML per install.
- Widening the bind host to `0.0.0.0` is an explicit, advanced-user action;
  authentication is still required on the listener, and `AGPEER_UI_TOKEN_INJECT`
  must not be enabled on an unauthenticated network path.

## API authentication

- A strong bearer token is **generated on first boot** and is **required on
  all requests, including loopback**.
- The token is persisted in the private data directory with restrictive file
  permissions; its location is printed at startup, its value is never written
  to logs.
- Local clients (the Tauri shell) read the token and auto-inject it.
- Requests without a valid token answer `401 AuthenticationFailed`.

**Browser token bootstrap.** In `webui` builds the core serves
`GET /__agpeer_token` so the in-browser UI can authenticate. It is
**loopback-only**: any non-loopback peer receives `403 PermissionDenied`.
Because the endpoint is unauthenticated by necessity, it exists only in
`webui` builds and only on the loopback interface.

For containers/LAN use, `AGPEER_UI_TOKEN_INJECT=1` makes the served page embed
`window.__AGPEER_TOKEN__` instead. **This is a widening-exposure setting**: any
client that can reach the HTTP port can read the API token. Use it only on
private ports/networks (see the compose example, which maps 41000 to the
loopback). Prefer the loopback token bootstrap wherever the browser shares the
host (the local one-binary story).

## Secrets

- Soulseek credentials, API keys, cookies, and auth headers are **never**
  committed and **never** logged.
- Secrets are stored via the platform secure storage abstraction: Windows
  DPAPI primary, `keyring` fallback.
- Diagnostics redact secrets; never return raw internal detail over the API.

## Filesystem

- All user-supplied paths are **canonicalized**.
- Configured **download roots are enforced**; `..` traversal is prevented.
- **Archive contents are treated as hostile**: extraction entries are
  sanitized against path traversal and symlink escapes in our code, before the
  external extractor runs.
- Destructive operations avoid following unexpected symlinks.
- Deleting files outside application-owned temporary directories requires
  **explicit opt-in** (e.g. `delete_data=true` on cancel/DELETE, never the
  default).

## Installer execution

Downloaded executables are untrusted. `run_installer` is privileged:

- never automatic, never implied by extraction;
- requires an explicit request plus a `confirmation_token` against a
  configurable confirmation policy;
- remote API access cannot launch executables unless separately enabled;
- every launch records an audit event;
- DRM bypass, crack application, license-key generation, and binary patching
  are not implemented as first-party features.

## Magnet search

Magnet search (`crates/hook`) is search-only: discovered magnets are pulled
through the normal torrent backend like any other caller-supplied source.

- **Built-in by default, domain-neutral.** With no `command` configured, the
  backend runs a generic search-engine query scoped to user‑configured
  `hook_search.domains` and optional `hook_search.sites` templates. No site
  name, indexer, or scraper logic is compiled in (enforced by a CI
  domain-neutrality guard); users own their config and responsibility.
- **External command override (optional).** A configured `[hook_search]
  command` runs instead. It executes **without a shell**; the search query is
  passed verbatim as a single argument (never interpolated) and each
  invocation is bounded by a configurable `timeout_secs`. The `{domains}`
  replacement/trailing-arguments behavior is `[hook_search]` only.
- **Explicit by default off.** The backend is always registered but only
  executes searches while the runtime `hook_search.enabled` setting is true.
- Site template searches fetch pages with a fixed user-agent and a bounded
  per-request timeout (the overall search is capped by `timeout_secs`); they
  never execute content, only extract magnet links.

## Audit

All significant actions (executable launch, destructive deletion, settings
changes that widen exposure) are recorded in the `events` audit table and are
observable via the event stream.

## Error handling

- Typed errors with stable codes; no raw stack traces over the public API.
- Detailed diagnostics remain in local logs only.
- Internal error detail is mapped to `Internal`/`Database`/`Backend` codes
  without leaking internals.
