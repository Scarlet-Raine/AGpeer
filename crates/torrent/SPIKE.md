# Torrent backend spike — librqbit (Phase 1, task 1)

Status: **embed `librqbit` as a library behind `TransferBackend`. The
`RqbitEngine` (feature `rqbit`, default off) is implemented against the
verified librqbit 8.1.1 API and compiles cleanly under
`cargo check/clippy --workspace --all-features`. The in-memory reference
engine (`MemoryEngine`) remains the default so the default build never
depends on librqbit.**

Research date: 2026-08-15. Crate examined: `librqbit` **8.1.1** (source in the
local cargo registry), upstream at `github.com/ikatson/rqbit`. License:
**Apache-2.0**. Primary crate has no semver commitment; sub-crates version
independently (`librqbit-core ^5`, `librqbit-bencode ^3.1`, `librqbit-dht
^5.3.1`, ...). Verify against current upstream docs before upgrading.

---

## 1. Verified API mapping (operation → librqbit 8.1.1 → status)

| Required operation | librqbit API (8.1.1) | Status |
|---|---|---|
| Add magnet URI | `AddTorrent::from_url(magnet)` → `Session::add_torrent` | Clean |
| Add local `.torrent` | `AddTorrent::from_local_filename(path)` (or `from_bytes`) | Clean |
| Add remote `.torrent` URL | `AddTorrent::from_url(url)`; `SUPPORTED_SCHEMES` includes `http:`/`https:`/`magnet:` | Clean |
| Resolve metadata | `ManagedTorrent::with_metadata(|m| m.info / m.file_infos)` | Clean |
| File selection pre-download | `AddTorrentOptions::only_files: Option<Vec<usize>>` (indices in torrent order, validated) | Clean |
| List / get | `Session::with_torrents(|it| ...)` / `Session::get(TorrentIdOrHash::Id(id))` | Clean |
| Progress / bytes | `ManagedTorrent::stats()` → `TorrentStats { progress_bytes, total_bytes, file_progress: Vec<u64>, finished, state }` | Clean |
| Peers | `stats.live.snapshot.peer_stats.live` (aggregate count, `usize`) | Clean |
| Download/upload speed | `LiveStats.download_speed.mbps` / `upload_speed.mbps` — **`f64` mebibytes/s; convert with `mbps * 1024 * 1024`** (there is no `as_bytes()` in 8.1.1) | Clean, conversion applied |
| ETA | computed ourselves from `(total - completed) / rate` (`LiveStats.time_remaining` is wrapped in a private `DurationWithHumanReadable`) | Clean |
| Pause / resume | `Session::pause(&handle)` / `Session::unpause(&handle)` | Clean |
| Cancel (+delete files) | `Session::delete(id: TorrentIdOrHash, delete_files: bool)` | Clean |
| Rate limits | `SessionOptions::ratelimits: LimitsConfig { upload_bps, download_bps: Option<NonZeroU32> }` | Clean |
| Listen port | `SessionOptions::listen_port_range: Option<Range<u16>>` (no `ListenerOptions` in 8.1.1) | Clean |
| DHT on/off | `SessionOptions::disable_dht: bool` (+ optional `dht_config: PersistentDhtConfig`) — no `dht: Option<DhtSessionConfig>` field | Clean |
| Tracker on/off | per-add `AddTorrentOptions::disable_trackers: bool` (not a session option) | Clean |
| LSD on/off | **No session toggle in 8.1.1.** LSD follows rqbit defaults | Limitation — `enable_lsd` is accepted but not honored by the rqbit engine |
| Persistence / fast resume | `SessionOptions::persistence: Option<SessionPersistenceConfig::Json { folder }>` + `fastresume: bool` | Clean, deferred to storage crate wave |
| Private torrents | `m.info.private` (`TorrentMetaV1Info.private`); rqbit disables peer discovery/PEX internally | Clean |
| PEX explicit toggle | **None.** PEX is on by default; `TorrentConfig.enable_pex` is accepted but ignored by the rqbit engine | Limitation |

## 2. Ergonomics / instability findings

- `Session::new`/`new_with_opts` return `BoxFuture<'static, anyhow::Result<Arc<Session>>>`;
  `Session::add_torrent` takes `self: &Arc<Self>` and returns a `BoxFuture` tied
  to that borrow. Callable directly, but the signatures are unusual for a library.
- Torrent ids are `usize` (`pub type TorrentId = usize`); the normalized model
  keeps its own opaque UUIDs and maps them in `RqbitEngine`'s private registry.
- Only ~12% of the crate is rustdoc-documented; the source is readable and the
  API surface used here is stable across the recent 8.x releases, but there is
  **no semver guarantee** and the crate releases often.
- `librqbit` 8.x is `edition = "2024"` → **requires Rust ≥ 1.85**. The agpeer
  workspace advertises rust-version 1.80, so the feature must only be enabled on
  a toolchain that satisfies librqbit.
- Default features pull `default-tls` (reqwest) and `http-api-client`; the HTTP
  server (`http-api`, axum) is off by default — good for embedding.
- Build cost is high (heavy dependency tree: reqwest, dht, upnp, rustls, ...);
  the shared workspace target dir serializes on the cargo lock.

## 3. Decision

**Embed `librqbit` as a library** — decision #1 in the implementation plan holds.
Every v1 operation (add magnet/.torrent/URL, list/get with progress/peers/
speeds/ETA, pause/resume/cancel, file selection, rate limits, listen port, DHT/
tracker toggles, private-torrent handling) is achievable through the current API,
and rqbit is a mature, actively maintained Apache-2.0 codebase.

**Phase-1 delivery:** the `RqbitEngine` is:

1. implemented against the verified 8.1.1 API (`crates/torrent/src/rqbit.rs`),
2. gated behind the **`rqbit` cargo feature (default off)**,
3. compile-verified: `cargo clippy --workspace --all-targets --all-features
   -- -D warnings` and `cargo test --workspace --all-features` both pass,
4. **not** wired as the default: `TorrentBackend::new` uses the fully tested
   in-memory reference engine (`MemoryEngine`), and the feature-gated
   `TorrentBackend::new_rqbit` wires the real one.

**Fallback (per plan decision #1):** if a future librqbit release breaks the
API used here, the fallback is to run `rqbit` as a sidecar process behind the
same `TransferBackend` — no core changes needed.

## 4. Verification checklist

```text
# toolchain: Rust >= 1.85 (librqbit is edition 2024)
cargo check -p agpeer-torrent --features rqbit          # PASS (2026-08-15)
cargo clippy --workspace --all-targets --all-features -- -D warnings  # PASS
cargo test --workspace --all-features                    # PASS (99 tests)
# remaining: live smoke test against a legal local fixture (no public network):
#   add a .torrent with --features rqbit, list/get, pause/resume, cancel
```

## 5. Default build (no feature)

`cargo test -p agpeer-torrent` / `cargo clippy -p agpeer-torrent --all-targets`
do **not** compile librqbit (feature off). All memory-engine tests, source
validation, bencode parsing, file selection, private-flag and state-mapping
tests run without it.
