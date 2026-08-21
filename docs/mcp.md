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

## Tools

The tool surface mirrors the agent-facing operations in [api.md](api.md):

| Tool | REST endpoint |
|---|---|
| `status` | `GET /status` |
| `list_backends` | `GET /backends` |
| `list_transfers` | `GET /transfers` |
| `get_transfer` | `GET /transfers/{id}` |
| `add_transfer` | `POST /transfers` |
| `pause_transfer` | `POST /transfers/{id}/pause` |
| `resume_transfer` | `POST /transfers/{id}/resume` |
| `cancel_transfer` | `POST /transfers/{id}/cancel` |
| `delete_transfer` | `DELETE /transfers/{id}` |
| `list_transfer_files` | `GET /transfers/{id}/files` |
| `list_searches` | `GET /searches` |
| `start_search` | `POST /searches` |
| `get_search` | `GET /searches/{id}` |
| `get_search_results` | `GET /searches/{id}/results` |
| `stop_search` | `POST /searches/{id}/stop` |
| `download_search_result` | `POST /searches/{sid}/results/{rid}/download` |
| `list_postprocess_jobs` | `GET /postprocess` |
| `get_postprocess_job` | `GET /postprocess/{id}` |
| `create_postprocess_job` | `POST /postprocess` |
| `get_settings` | `GET /settings` |

`create_postprocess_job` requires `confirmation_token` only when the job plan
includes the privileged `run_installer` step — matching the core's policy.
Unrestricted shell execution is **not** exposed.

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

## Security notes

- The MCP server only talks to the loopback-local agpeer core; it never opens
  a network listener of its own.
- It never logs the bearer token and never returns core secrets (the core
  already redacts them from `/settings`).
- Privileged and destructive operations (`delete_transfer`, `cancel_transfer`,
  `create_postprocess_job` with `run_installer`) retain the core's explicit
  confirmation semantics.

## Development

```bash
cargo fmt -p agpeer-mcp -- --check
cargo clippy -p agpeer-mcp --all-targets --all-features -- -D warnings
cargo test -p agpeer-mcp
```

Integration tests in `crates/mcp/tests/client.rs` exercise the REST client
against a mock agpeer API (via `httpmock`) so no live core or network is
required in CI.
