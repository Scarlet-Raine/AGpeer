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

## API authentication

- A strong bearer token is **generated on first boot** and is **required on
  all requests, including loopback**.
- The token is persisted in the private data directory with restrictive file
  permissions; its location is printed at startup, its value is never written
  to logs.
- Local clients (the Tauri shell) read the token and auto-inject it.
- Requests without a valid token answer `401 AuthenticationFailed`.

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

## Hook search execution

`[hook_search]` runs a user-configured external command to discover magnet links:

- **Explicit by default off.** The backend only runs when `[hook_search].enabled`
  is true and a `command` is configured.
- The command runs **without a shell**; the search query is passed verbatim as a
  single argument (never interpolated) and each invocation is bounded by a
  configurable `timeout_secs`.
- Hook results are **search-only** — they never execute downloads. Magnets
  returned are pulled through the normal torrent backend path like any other
  caller-supplied source.

## Audit

All significant actions (executable launch, destructive deletion, settings
changes that widen exposure) are recorded in the `events` audit table and are
observable via the event stream.

## Error handling

- Typed errors with stable codes; no raw stack traces over the public API.
- Detailed diagnostics remain in local logs only.
- Internal error detail is mapped to `Internal`/`Database`/`Backend` codes
  without leaking internals.
