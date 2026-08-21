//! `agpeer-mcp` binary — serves the MCP protocol over stdio.
//!
//! Coding agents (Claude Code, Kilo, Cursor, ...) launch this process and talk
//! to it over stdin/stdout. It forwards every tool call to the agpeer core
//! REST API, authenticating with a bearer token resolved from CLI args or the
//! environment.

use std::path::{Path, PathBuf};

use agpeer_mcp::{AgpeerClient, AgpeerServer};
use clap::Parser;
use rmcp::transport::stdio;
use rmcp::ServiceExt;

#[derive(Parser)]
#[command(
    name = "agpeer-mcp",
    version,
    about = "MCP server bridging coding agents to the agpeer REST API"
)]
struct Cli {
    /// Base URL of the agpeer core API.
    #[arg(long, default_value = "http://127.0.0.1:41000")]
    api_base: String,

    /// Bearer token for the agpeer API (overrides --token-file / --data-dir).
    #[arg(long, conflicts_with = "token_file", conflicts_with = "data_dir")]
    token: Option<String>,

    /// Path to a file containing the bearer token.
    #[arg(long)]
    token_file: Option<PathBuf>,

    /// agpeer data dir; reads `<dir>/token` when no token is given.
    #[arg(long)]
    data_dir: Option<PathBuf>,
}

fn read_token_file(path: &Path) -> Result<String, String> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read token file {}: {e}", path.display()))?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        Err(format!("token file {} is empty", path.display()))
    } else {
        Ok(trimmed.to_string())
    }
}

fn resolve_token(cli: &Cli) -> Result<String, String> {
    if let Some(t) = &cli.token {
        return Ok(t.clone());
    }
    if let Some(f) = &cli.token_file {
        return read_token_file(f);
    }
    if let Some(d) = &cli.data_dir {
        return read_token_file(&d.join("token"));
    }
    if let Ok(t) = std::env::var("AGPEER_TOKEN") {
        if !t.trim().is_empty() {
            return Ok(t);
        }
    }
    if let Some(f) = std::env::var_os("AGPEER_TOKEN_FILE") {
        return read_token_file(Path::new(&f));
    }
    Err(
        "no agpeer auth token: pass --token, --token-file, --data-dir, or set AGPEER_TOKEN / \
         AGPEER_TOKEN_FILE"
            .to_string(),
    )
}

async fn run(cli: Cli) -> Result<(), String> {
    let token = resolve_token(&cli)?;
    let server = AgpeerServer::new(AgpeerClient::new(&cli.api_base, token));

    // The core service is a separate process; poke it once so a misconfigured
    // API base / token fails loudly before the agent handshake.
    if let Err(e) = server.client().status().await {
        return Err(format!(
            "cannot reach agpeer core at {}: {e}",
            server.client().base_url()
        ));
    }

    let service = server
        .serve(stdio())
        .await
        .map_err(|e| format!("failed to start MCP server: {e}"))?;
    let ct = service.cancellation_token();
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    tokio::select! {
        _ = service.waiting() => {}
        _ = &mut ctrl_c => {
            ct.cancel();
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli).await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
