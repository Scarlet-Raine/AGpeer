# Debugging MCP server (`agpeer-debug-mcp`)

A second MCP server for **fast, low-token debugging**. While `agpeer-mcp`
drives the agpeer REST API, `agpeer-debug-mcp` inspects the repository and the
agpeer core's logs locally and returns **counts and capped, context-limited
snippets** — not raw dumps — so a coding agent spends fewer tokens finding a
bug.

## How it works

```
calling agent ⇄ MCP (stdio) ⇄ agpeer-debug-mcp ⇄ local repo + agpeer log files
```

- The server is stateless and filesystem-local. It never opens a network
  listener.
- Two roots configure it (`--root` for source/git, `--log-dir` for logs).
- Every tool enforces hard caps (`MAX_RESULTS`, `MAX_CHARS`) and reports
  match/file counts so the agent can decide whether to dig deeper.

## Build & run

```bash
cargo build --release -p agpeer-debug-mcp
# binary: target/release/agpeer-debug-mcp.exe
```

```text
agpeer-debug-mcp [--root <DIR>] [--log-dir <DIR>]
```

- `--root` defaults to the current directory.
- `--log-dir` resolution: `--log-dir`, then `AGPEER_LOG_DIR`, then
  `<root>/run/data/logs` (the agpeer core's runtime log dir).

The core log writer keeps `agpeer.log` at 2,000 lines and retains up to 20
files total (`agpeer.log` plus `.1` through `.19`). The debug MCP discovers all
of these files automatically for `log_tail` and `log_search`.

Serve only via stdio (agents launch it as a subprocess).

## Tools

| Tool | Purpose | What it returns |
|---|---|---|
| `runtime_info` | Active roots | repo root, log dir, latest log file + size |
| `log_tail` | Tail latest agpeer log | last N lines, optional substring/level filter |
| `log_search` | Regex-search all logs | capped matches + context, match/file counts |
| `code_grep` | Regex-search source | capped matches + context (skips `target`/`node_modules`/`.git`) |
| `code_read` | Read a file | path + a line range (default whole file, capped) |
| `code_files` | List repo files | capped list + indexed count |
| `code_symbol` | Find a definition | `file:line` + snippet for fn/struct/enum/trait/impl/… |
| `git_status` | Working tree | branch, last commit, `git status --short` |
| `git_log` | Recent commits | oneline log |
| `git_diff` | Diff | `--stat` summary, or full diff when requested |

### Token-saving behaviors

- `log_tail` accepts `contains`/`level` to narrow before returning.
- `log_search`/`code_grep` cap results and print `-- N total matches, N shown`.
- `code_read` reads a specific range instead of a whole file.
- `code_symbol` returns a short snippet around the definition.
- All outputs are byte-capped and truncated with a marker.

## Security

- Confined to the configured `root`: `code_read` rejects paths that escape the
  root (canonicalized). A root must be a directory.
- Build/artifact dirs (`target`, `node_modules`, `.git`, `dist`, `build`, …)
  are never walked, so search is fast and never returns build noise.
- Read-only: nothing writes to the repo (except creating the log dir if absent).
- Git commands run in `root` only and are read-only.

## Configure a coding agent

Point the agent at the binary, the repo root, and the agpeer log dir. The
repo root already ships ready-made configs for Kilo (`kilo.json` →
`mcp.agpeer-debug`), OpenAI Codex CLI (`.codex/config.toml` →
`mcp_servers.agpeer-debug`), Claude Code / generic clients (`.mcp.json` →
`mcpServers.agpeer-debug`), and Cursor (`.cursor/mcp.json`). Adjust the
absolute binary/root paths if your checkout lives elsewhere.

```json
{
  "mcpServers": {
    "agpeer-debug": {
      "command": "D:\\dev\\agpeer\\target\\release\\agpeer-debug-mcp.exe",
      "args": ["--root", "D:\\dev\\agpeer", "--log-dir", "D:\\dev\\agpeer\\run\\data\\logs"]
    }
  }
}
```

Combine it with the API MCP server ([mcp.md](mcp.md)) so the same agent can
both debug (this server) and act on jobs (the API server).

## Development

```bash
cargo fmt -p agpeer-debug-mcp -- --check
cargo clippy -p agpeer-debug-mcp --all-targets --all-features -- -D warnings
cargo test -p agpeer-debug-mcp
```
