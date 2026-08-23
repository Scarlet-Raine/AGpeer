# MCP server

`agpeer-mcp` is a thin Model Context Protocol server that lets coding agents
(Claude Code, Kilo, Cursor, ...) drive agpeer through the same stable
`/api/v1` REST API the desktop UI uses.

It carries **no business logic of its own** — every MCP tool maps one-to-one
onto a documented REST endpoint. See [api.md](api.md) for the endpoint
contract and the AGENTS.md "MCP" section for the design rules.

## How it works

```
calling agent ⇄ MCP (stdio, newline-delimited JSON-RPC) ⇄ agpeer-mcp ⇄ REST /api/v1 ⇄ agpeer core
```

- `agpeer-mcp` speaks the MCP protocol over **stdio** (stdin/stdout). Agents
  launch it as a subprocess.
- For each tool call it makes an authenticated HTTP request to the agpeer core
  (default `http://127.0.0.1:41000`) using the bearer token.
- The bearer token is read from CLI args, a token file, or the environment —
  see [Run](#run). It is never embedded in the MCP server.

## Build

The crate is part of the workspace. Requiring Rust 1.88+ (rmcp's MSRV):

```bash
cargo build --release -p agpeer-mcp
# binary: target/release/agpeer-mcp.exe
```

## Run

```text
agpeer-mcp [--api-base <URL>] [--token <TOKEN> | --token-file <PATH> | --data-dir <DIR>]
```

Token resolution order:

1. `--token <TOKEN>`
2. `--token-file <PATH>` (file containing the token)
3. `--data-dir <DIR>` (reads `<DIR>/token`, the agpeer core's token file)
4. `AGPEER_TOKEN` environment variable
5. `AGPEER_TOKEN_FILE` environment variable (path to a token file)

The token file is where the agpeer core persists its API token at startup
(`<data_dir>/token`). For the shipped `run/` environment pass
`--data-dir D:\dev\agpeer\run\data`. The environment variables used by the
core (`AGPEER_CONFIG`, `AGPEER_SOULSEEK_*`) do **not** control the MCP server;
pass its token/URL explicitly.

On startup the MCP server pings `GET /api/v1/status` so a wrong API base or
token fails loudly before the agent handshake.

## Configuring a coding agent

The agent launches `agpeer-mcp` as a stdio MCP server. Point it at your agpeer
core and its token file.

Ready-made, machine-local configs are checked into the repo root for the
common agents (edit the absolute paths if your checkout lives elsewhere):

| Agent | File | Server key |
|---|---|---|
| Kilo | `kilo.json` (`mcp.agpeer`) | `agpeer` |
| OpenAI Codex CLI | `.codex/config.toml` (`mcp_servers.agpeer`) | `agpeer` |
| Claude Code / generic MCP clients | `.mcp.json` (`mcpServers.agpeer`) | `agpeer` |
| Cursor | `.cursor/mcp.json` (`mcpServers.agpeer`) | `agpeer` |

Each lists the same server under the name `agpeer`; the separate debug server
(`agpeer-debug`) is configured alongside in the same files.

### Claude Code / generic MCP clients (`.mcp.json`)

```json
{
  "mcpServers": {
    "agpeer": {
      "command": "D:\\dev\\agpeer\\target\\release\\agpeer-mcp.exe",
      "args": ["--api-base", "http://127.0.0.1:41000", "--data-dir", "D:\\dev\\agpeer\\run\\data"],
      "type": "stdio"
    }
  }
}
```

Remove `--data-dir` and instead rely on `AGPEER_TOKEN` if you already export it.

### Cursor (`.cursor/mcp.json`)

```json
{
  "mcpServers": {
    "agpeer": {
      "command": "D:\\dev\\agpeer\\target\\release\\agpeer-mcp.exe",
      "args": ["--api-base", "http://127.0.0.1:41000", "--data-dir", "D:\\dev\\agpeer\\run\\data"]
    }
  }
}
```

### Environment variable alternative

```text
AGPEER_TOKEN=<core bearer token>   (or AGPEER_TOKEN_FILE=<path to token file>)
```

and no `--token`/`--token-file`/`--data-dir` args.

## Tool call conventions

Every tool returns an MCP text content whose single `text` field is the
**pretty-printed JSON of the underlying REST response**. Parse
`result.content[0].text` as JSON:

```json
{
  "jsonrpc": "2.0",
  "id": 7,
  "result": {
    "content": [
      { "type": "text", "text": "{ ...REST response JSON... }" }
    ],
    "isError": false
  }
}
```

Failures come back as MCP protocol errors (`isError`/error object) whose
message embeds the REST status and body, e.g.
`agpeer responded with 404: {"code":"TransferNotFound",...}`.

Raw JSON-RPC over stdio (newline-delimited) — initialize handshake once, then
one line per call:

```jsonc
// 1. handshake (once, on connect)
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"my-agent","version":"0.1.0"}}}
{"jsonrpc":"2.0","method":"notifications/initialized"}

// 2. every tool call looks like this
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"<tool>","arguments":{ ... }}}
```

Most agents never see this layer — their MCP client handles it. The
templates below show only the `"arguments"` object per tool.

## Tool reference

### Health & settings

#### `status` — no arguments

```jsonc
{"name":"status","arguments":{}}
// → { "version":"0.1.0", "uptime_secs":1234, "db":"ok",
//     "server_time":"...", "backends":[ {"backend":"soulseek",
//       "search_available":true,"transfer_available":true,"state":"ready"} ] }
```

Check `backends[].transfer_available` / `search_available` before relying on a
backend. Call this first if any other tool errors.

#### `list_backends`, `list_searches`, `list_transfers`, `list_postprocess_jobs`, `get_settings`, `list_library` — no arguments

Return arrays/maps of the corresponding resources. `get_settings` returns the
runtime settings map with secrets redacted by the core. `list_library` lists
files under the configured `[postprocess].library_root` (empty when none is
configured).

### Torrent transfers

Torrents are driven through the transfer lifecycle tools. Sources accepted by
`add_transfer` with `backend:"torrent"`:

- a `magnet:` URI,
- a path to a local `.torrent` file,
- an `http(s)://` URL to a remote `.torrent`.

The destination must stay inside the configured download roots; anything else
is rejected (`UnsafePath`). An unparseable source is rejected with
`InvalidSource`.

Transfer lifecycle (`state` on the transfer object):

```text
queued → resolving → downloading → completed
                    ↘ failed        (paused ⇄ downloading)
```

- `resolving` means the magnet is fetching metadata from peers; large/swarm
  -poor magnets can sit here for a while before data flows.
- Terminal states: `completed`, `failed`, `cancelled`. `error` carries a
  short message when `failed`.
- Progress: poll `get_transfer`; `progress` is 0..1 alongside
  `bytes_completed`/`bytes_total`. The webui's live stream comes from SSE
  `/api/v1/events`, which is not exposed as an MCP tool — agents poll every
  ~3–4 s instead of tight-looping.

Multi-file torrents:

1. Add the transfer, then call `list_transfer_files` `{id}` to see the file
   list with per-file selection state.
2. To change what downloads, re-add the transfer with
   `file_selection:[{"index":"0","selected":true},{"index":"2","selected":false},...]`
   (indices are strings). Cancel the first attempt with
   `cancel_transfer {id, delete_data:true}` if needed.

Optional discovery: if a `hook` magnet-search backend is configured and
enabled, `start_search {backend:"hook", query:"..."}` returns results whose
`backend_metadata.magnet` holds a ready-to-use magnet URI — pass it straight
to `add_transfer`. See [Search](#search).

Failure playbook (torrent):

- `add_transfer` answers `InvalidSource` → the magnet/`.torrent` is malformed;
  ask the user for a corrected link or try another result.
- Stuck in `resolving` for several minutes → the swarm is likely dead; report
  and suggest another source rather than waiting indefinitely.
- Stalled `downloading` (progress not moving across polls) → check
  `list_transfers` for other activity, then report; there is no force-recheck
  tool in v1.
- `failed` with an error string → surface `error` verbatim; do not retry the
  same source automatically unless the user asks.
- Cleanup only on request: `delete_transfer {id, delete_data?}`.

### Search

#### `start_search`

| Argument | Type | Required | Notes |
|---|---|---|---|
| `backend` | string | yes | `"soulseek"` or `"hook"` (user-configured magnet-search backend) |
| `query` | string | yes | free-text query, e.g. `"artist album flac"` |
| `user` | string | no | restrict to one peer (soulseek only) |
| `extension` | string | no | e.g. `"flac"`, `"mp3"` (soulseek only) |
| `min_size` | integer | no | bytes (soulseek only) |
| `max_results` | integer | no | cap |

```jsonc
{"name":"start_search","arguments":{"backend":"soulseek","query":"radiohead","extension":"flac"}}
// → { "search_id":"0df84120-..." }
```

Searches run asynchronously server-side; results accumulate as peers respond.
Poll `get_search_results` once after ~10–15 seconds rather than spamming —
most results arrive in the first wave, with stragglers over the next minute.

Hook searches return magnet results. Each result carries:

- `backend_metadata.magnet` and `attributes.magnet` — the ready-to-use magnet
  URI; pass it straight to `add_transfer` with `backend:"torrent"`.
- `attributes.seeders` / `attributes.leechers` — when the source provides
  them; prefer results with more seeders.
- `filename` — a display title derived from the magnet or page.

Hook search must be enabled in runtime settings (`hook_search.enabled`) or
searches answer `503 BackendUnavailable`.

#### `get_search_results`

| Argument | Type | Required |
|---|---|---|
| `id` | string (search id) | yes |

Returns a **bare JSON array** of results (not wrapped in an object):

```jsonc
[ {
    "result_id":"d556acc1-...",
    "search_id":"0df84120-...",
    "username":"tucker97123",
    "path":"/",
    "filename":"@@ipiaz\\sidify\\Radio Muse\\Radiohead\\OK Computer\\song.mp3",
    "size":8578805,
    "extension":null,
    "bitrate":256,
    "duration":264,
    "queue_length":43,
    "free_upload_slots":true,
    "upload_speed":53157235,
    "backend_metadata":{"soulseek":{"filename":"@@ipiaz\\sidify\\...","token":1,"username":"tucker97123"}}
} ]
```

Notes:

- `filename` preserves the *peer's* share-index separators (often
  backslashes); use it verbatim when matching or displaying. Result ids are
  stable UUIDv5 keys derived from username+filename.
- `free_upload_slots=true` means the peer should answer immediately;
  `queue_length` is their upload queue depth.

#### `get_search`, `stop_search` — `{"id":"<search id>"}`

Fetch a search's status row / stop collection early.

### Downloading

#### `download_search_result`

| Argument | Type | Required | Notes |
|---|---|---|---|
| `search_id` | string | yes | from `start_search` |
| `result_id` | string | yes | from `get_search_results` |
| `destination` | string | no | absolute directory; defaults to the configured soulseek download root |

```jsonc
{"name":"download_search_result","arguments":{
  "search_id":"0df84120-...",
  "result_id":"d556acc1-...",
  "destination":"E:\\Media\\Music\\unsorted"
}}
// → { "transfer_id":"efad7459-..." }
```

Behavior contract (validated live):

- Free-slot peers typically deliver within seconds; busy peers leave the
  transfer `queued` while waiting for their slot.
- Peers behind firewalls relay the file through the core automatically; no
  port forwarding is required on your side for downloads.
- A peer may silently ignore the request or refuse it later; refusals flip
  the transfer to `failed` with error `download refused by peer`. There is no
  automatic retry — pick a different result/peer and call again.
- Downloads do not resume; a cancelled/killed transfer restarts from zero.

#### Transfer objects (`get_transfer`, `list_transfers`)

Key fields: `id`, `backend` (`"torrent"|"soulseek"`), `source`,
`display_name`, `state` (`queued|downloading|paused|completed|failed|cancelled`),
`progress` (0..1), `bytes_completed`, `bytes_total`, `destination`, `error`
(string or null), `created_at`, `completed_at`, `postprocess_state`,
`metadata` (opaque backend details).

Poll `get_transfer` every ~3–4 s until `state` is terminal
(`completed|failed|cancelled`). Example:

```jsonc
{"name":"get_transfer","arguments":{"id":"efad7459-ecf1-4fe1-b6b2-ce6d9349a256"}}
// → { "id":"...", "state":"completed", "progress":1.0,
//     "bytes_completed":4822771, "bytes_total":4822771,
//     "destination":"E:\\Media\\Music\\unsorted", "error":null, ... }
```

#### Lifecycle tools

| Tool | Arguments | Effect |
|---|---|---|
| `pause_transfer` | `{"id"}` | torrent only; soulseek rejects pause/resume in v1 |
| `resume_transfer` | `{"id"}` | torrent only |
| `cancel_transfer` | `{"id","delete_data"?}` | stop; `delete_data:true` also deletes partial/full files (default false) |
| `delete_transfer` | `{"id","delete_data"?}` | remove the job from the list; optional data delete |
| `add_transfer` | see below | add a torrent/soulseek transfer directly |
| `list_transfer_files` | `{"id"}` | per-file selection view (torrent multi-file) |

`add_transfer` arguments:

| Argument | Type | Required | Notes |
|---|---|---|---|
| `backend` | string | yes | `"torrent"` or `"soulseek"` |
| `source` | string | yes | magnet URI, `.torrent` path/URL, or a `soulseek:` result id |
| `destination` | string | no | absolute directory |
| `display_name` | string | no | |
| `file_selection` | array | no | `[{"index":"0","selected":true}]` (torrent) |
| `metadata` | object | no | passthrough key/value pairs; see post-process routing below |

Post-process routing hints (metadata): pass
`"metadata":{"postprocess":{"media":"anime"}}` to route into the anime tree,
`"media":"tv"`/`"movie"` to force series/film routing, and
`"metadata":{"postprocess":{"music_path":"Artist/Album"}}` to drop an audio
download at an exact subfolder under `Music/`. Without hints, filenames are
routed heuristically (`SxxEyy` → TV, year → Movies). See
[postprocessing.md](postprocessing.md#media-organization).

Prefer `download_search_result` for soulseek results found via search; it is
the normal entry point and wires up all bookkeeping.

### Post-processing

Post-processing jobs are created by the **core** (auto-organize on completed
transfers when `[postprocess].auto_organize` is enabled). There is no manual
"create job" tool; agents observe jobs instead:

| Tool | Arguments | Effect |
|---|---|---|
| `list_postprocess_jobs` | none | list all jobs |
| `get_postprocess_job` | `{"id"}` | one job with per-step states |

Job state also shows up as `postprocess_state` on the transfer object, and
`postprocess.*` events drive the UI's SSE stream.

### Library

#### `list_library` — no arguments

Lists files/directories under the configured `[postprocess].library_root`,
directories first. Returns an empty array when no library root is configured.

```jsonc
// → [ { "path":"TV Shows/Show/ep.mkv", "absolute_path":"E:\\Media\\TV Shows\\Show\\ep.mkv",
//       "size":1234, "is_dir":false } ]
```

### Settings

Runtime settings live in the core's SQLite store; secrets are never readable
or settable through this API. Static bootstrap values (ports, paths) are
configured in the TOML file, not here.

| Tool | Arguments | Effect |
|---|---|---|
| `get_settings` | none | full redacted settings map |
| `put_settings` | `{"settings":{...}}` | set several keys at once; returns the updated map |
| `get_setting` | `{"key"}` | fetch one setting |
| `put_setting` | `{"key","value"}` | set one setting to any JSON value |
| `delete_setting` | `{"key"}` | remove the override, restoring the default |

```jsonc
{"name":"put_setting","arguments":{"key":"hook_search.enabled","value":true}}
```

Useful keys: `hook_search.enabled`, `hook_search.domains`,
`hook_search.sites` — see [api.md](api.md#settings) for their semantics.

## Recipe: pull a song end to end

1. `status` — confirm soulseek shows `transfer_available:true`.
2. `start_search` `{backend:"soulseek", query:"<what the user asked for>"}`.
3. Wait ~12 s; `get_search_results` `{id}`.
4. Pick a candidate: prefer `free_upload_slots:true`, smallest plausible
   `size` first, skip peers with huge `queue_length`.
5. `download_search_result` `{search_id, result_id, destination:"<dir>"}`.
6. Poll `get_transfer` every 3–4 s up to ~90 s. On `completed`, report
   `destination`. If still `queued`, either wait longer (busy peer) or start a
   second download from a different peer — multiple parallel transfers are fine.
7. Only clean up with `delete_transfer` `{"id","delete_data":true}` when the
   user asks to remove something.

Failure playbook:

- Peer silent for minutes → try another result (different `username`).
- `failed` + `download refused by peer` → the peer rejected it; choose another.
- Backend unavailable → re-check `status`; the core may need restarting.

## Recipe: download a torrent end to end

1. `status` — confirm the torrent backend shows `transfer_available:true`.
2. Obtain a source:
   - the user supplies a magnet/`.torrent` path/URL, **or**
   - `start_search` `{backend:"hook", query:"..."}`, wait ~10 s, then
     `get_search_results {id}` and pick the result with the most
     `attributes.seeders`; take `backend_metadata.magnet`.
3. `add_transfer`
   `{backend:"torrent", source:"<magnet>", destination:"<configured download dir>"}`.
   The response is `{ "transfer_id":"..." }`.
4. For multi-file torrents, `list_transfer_files` `{id}`; if the user wants a
   subset, cancel with `delete_data:true` and re-add with `file_selection`.
5. Poll `get_transfer` `{id}` every 3–4 s until `state` is terminal. Report
   progress only on meaningful change (e.g. every 25% or state transition),
   not per poll.
6. On `completed`: report the `destination`. With auto-organize enabled the
   core moves files into the library tree (`<root>/<tv_dir|anime_dir|...>`);
   organized paths are queryable via `list_library` and `postprocess_state`
   on the transfer. Set `metadata.postprocess.media` at add time when you
   know the routing (e.g. `"anime"`) or `music_path` for music albums.
7. Never delete data unless the user asks (`delete_transfer`,
   `cancel_transfer`).

### Ready-to-paste agent instructions

Hand this block to a personal coding assistant that has the `agpeer` MCP
server configured:

```text
You can operate my agpeer client through its MCP tools.

Ground rules:
- Call `status` first; if a needed backend is not transfer_available, stop and tell me.
- Torrents: use add_transfer {backend:"torrent", source:<magnet|.torrent path|URL>,
  destination:<my download dir>}. Never invent destinations outside dirs I named.
- Poll get_transfer every ~4 seconds while waiting; do not spam other calls.
  Stop polling at completed|failed|cancelled and report the outcome plus the
  destination directory.
- Multi-file torrents: call list_transfer_files first and ask me which files
  to keep before downloading if the torrent is large.
- Sorting: set metadata.postprocess.media ("anime"|"tv"|"movie") at
  add_transfer time when you know the type, and metadata.postprocess.music_path
  ("<artist>/<album>") for music, so downloads land in the right library
  folder automatically.
- Never delete anything: no delete_transfer/cancel_transfer with delete_data
  unless I explicitly say so for a specific transfer id.
- If a transfer fails or stalls >5 minutes in resolving/downloading without
  progress, report the error field verbatim and ask me how to proceed.
- After completion, verify results with list_library when relevant.
```

## Security notes

- The MCP server only talks to the loopback-local agpeer core; it never opens
  a network listener of its own.
- It never logs the bearer token and never returns core secrets (the core
  already redacts them from `/settings`, and secrets are not settable there).
- Privileged and destructive operations (`delete_transfer`,
  `cancel_transfer`) retain the core's explicit confirmation semantics.
  Unrestricted shell execution is not exposed as a tool.

## Development

```bash
cargo fmt -p agpeer-mcp -- --check
cargo clippy -p agpeer-mcp --all-targets --all-features -- -D warnings
cargo test -p agpeer-mcp
```

Integration tests in `crates/mcp/tests/client.rs` exercise the REST client
against a mock agpeer API (via `httpmock`) so no live core or network is
required in CI.
