//! `agpeer-debug-mcp` binary — fast, low-token debugging over MCP stdio.
//!
//! Serves MCP on stdin/stdout. Tools read the repo root / log dir locally and
//! never open a network listener.

use std::path::{Path, PathBuf};

use agpeer_debug_mcp::DebugServer;
use clap::Parser;
use rmcp::transport::stdio;
use rmcp::ServiceExt;

#[derive(Parser)]
#[command(
    name = "agpeer-debug-mcp",
    version,
    about = "Fast, low-token debugging MCP server (logs, code search, git)"
)]
struct Cli {
    /// Repository root to inspect. Defaults to the current directory.
    #[arg(long)]
    root: Option<PathBuf>,

    /// Directory containing agpeer core logs. Defaults to
    /// `AGPEER_LOG_DIR` or `<root>/run/data/logs`.
    #[arg(long)]
    log_dir: Option<PathBuf>,
}

fn resolve_log_dir(cli: &Cli, root: &Path) -> PathBuf {
    if let Some(d) = &cli.log_dir {
        return d.clone();
    }
    if let Some(d) = std::env::var_os("AGPEER_LOG_DIR") {
        return PathBuf::from(d);
    }
    root.join("run").join("data").join("logs")
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let cli = Cli::parse();
    let root = std::env::current_dir()
        .map_err(|e| format!("cannot resolve current dir: {e}"))?
        .canonicalize()
        .map_err(|e| format!("cannot canonicalize root: {e}"))?;
    let root = if let Some(r) = &cli.root {
        r.canonicalize()
            .map_err(|e| format!("bad --root {}: {e}", r.display()))?
    } else {
        root
    };
    if !root.is_dir() {
        return Err(format!("root is not a directory: {}", root.display()));
    }
    let log_dir = resolve_log_dir(&cli, &root);
    std::fs::create_dir_all(&log_dir).ok();

    let server = DebugServer::new(root, log_dir);
    let service = server
        .serve(stdio())
        .await
        .map_err(|e| format!("failed to start MCP server: {e}"))?;
    let ct = service.cancellation_token();
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    tokio::select! {
        _ = service.waiting() => {}
        _ = &mut ctrl_c => { ct.cancel(); }
    }
    Ok(())
}
