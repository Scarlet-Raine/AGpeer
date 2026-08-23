//! agpeer core service binary: `serve` (HTTP API + backends) and `migrate`.

use agpeer_api::router;
use agpeer_common::Backend;
use agpeer_core::config::{default_data_dir, AppConfig};
use agpeer_core::housekeeping::{spawn_transfer_sync, spawn_ttl_sweeper};
use agpeer_core::state::AppState;
use agpeer_storage::Database;
use clap::{Parser, Subcommand};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

const LOG_LINES_PER_FILE: usize = 2_000;
const LOG_FILE_RETENTION: usize = 20;

/// Append-only log writer that rotates after a fixed number of newline-delimited
/// records. Retention includes the active file: `agpeer.log` plus
/// `agpeer.log.1` through `agpeer.log.19`.
struct LineRotatingWriter {
    path: PathBuf,
    file: Option<File>,
    lines: usize,
}

impl LineRotatingWriter {
    fn new(dir: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join("agpeer.log");
        let lines = count_lines(&path)?;
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let mut writer = Self {
            path,
            file: Some(file),
            lines,
        };
        if writer.lines >= LOG_LINES_PER_FILE {
            writer.rotate()?;
        }
        Ok(writer)
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file.as_mut().expect("log file is open").flush()?;
        // Drop the active handle before renaming it. This is required on
        // Windows, where an open file cannot be renamed.
        self.file.take();

        // Delete the oldest retained file, then shift newer files up one slot.
        if LOG_FILE_RETENTION > 1 {
            let oldest = rotated_path(&self.path, LOG_FILE_RETENTION - 1);
            let _ = std::fs::remove_file(oldest);
            for index in (1..LOG_FILE_RETENTION - 1).rev() {
                let from = rotated_path(&self.path, index);
                if from.exists() {
                    std::fs::rename(from, rotated_path(&self.path, index + 1))?;
                }
            }
            std::fs::rename(&self.path, rotated_path(&self.path, 1))?;
        }

        self.file = Some(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?,
        );
        self.lines = 0;
        Ok(())
    }
}

impl Write for LineRotatingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file
            .as_mut()
            .expect("log file is open")
            .write_all(buf)?;
        self.lines += buf.iter().filter(|&&byte| byte == b'\n').count();
        if self.lines >= LOG_LINES_PER_FILE {
            self.rotate()?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.as_mut().expect("log file is open").flush()
    }
}

fn rotated_path(active: &Path, index: usize) -> PathBuf {
    PathBuf::from(format!("{}.{}", active.display(), index))
}

fn count_lines(path: &Path) -> io::Result<usize> {
    if !path.exists() {
        return Ok(0);
    }
    let mut contents = Vec::new();
    File::open(path)?.read_to_end(&mut contents)?;
    Ok(contents.iter().filter(|&&byte| byte == b'\n').count())
}

#[derive(Parser)]
#[command(name = "agpeer", version, about = "agpeer core service")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Migrate {
        #[arg(long)]
        config: Option<PathBuf>,
    },
    Serve {
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

/// Initializes tracing with two layers:
///
/// - **Console**: concise `info`-level logs (agpeer + librqbit at info, noisy
///   third-party DEBUG at warn) so the terminal stays readable. Override with
///   `RUST_LOG`.
/// - **File**: FULL-detail logs (all targets at `debug` by default) written to
///   `<data_dir>/logs/agpeer.log` and rotated files `.1` through `.19` (2,000
///   lines per file, 20 files retained). Override the file level with
///   `AGPEER_LOG_FILE_FILTER` (e.g. `trace`).
fn init_tracing(log_dir: &Path) {
    let file_appender =
        LineRotatingWriter::new(log_dir).expect("failed to initialize rotating agpeer log writer");
    let (file_writer, file_guard) = tracing_appender::non_blocking(file_appender);
    let file_filter = EnvFilter::try_from_env("AGPEER_LOG_FILE_FILTER").unwrap_or_else(|_| {
        // Everything at debug into the file: full visibility for debugging.
        "debug".into()
    });
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_writer)
        .with_ansi(false)
        .with_filter(file_filter);

    let console_filter = EnvFilter::try_from_env("RUST_LOG").unwrap_or_else(|_| {
        "info,agpeer=info,librqbit=info,sqlx=warn,h2=warn,hyper_util=warn,reqwest=warn,\
         librqbit_dht=warn"
            .into()
    });
    let console_layer = tracing_subscriber::fmt::layer().with_filter(console_filter);

    tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer)
        .init();

    // Keep the non-blocking writer alive for the process lifetime.
    std::mem::forget(file_guard);
}

/// Best-effort data directory for logging, resolved from the CLI config before
/// the full app state is built.
fn resolve_data_dir(command: &Command) -> PathBuf {
    let config = match command {
        Command::Migrate { config } | Command::Serve { config } => match config {
            Some(path) => AppConfig::from_file(path),
            None => AppConfig::load(),
        },
    };
    config
        .map(|c| PathBuf::from(c.data_dir))
        .unwrap_or_else(|_| default_data_dir())
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    init_tracing(&resolve_data_dir(&cli.command).join("logs"));
    let result = match cli.command {
        Command::Migrate { config } => run_migrate(config).await,
        Command::Serve { config } => run_serve(config).await,
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run_migrate(config_override: Option<PathBuf>) -> Result<(), String> {
    let config = match config_override {
        Some(path) => AppConfig::from_file(&path).map_err(|e| e.to_string())?,
        None => AppConfig::load().map_err(|e| e.to_string())?,
    };
    let db = Database::open(config.db_path().to_str().ok_or("invalid db path")?)
        .await
        .map_err(|e| e.to_string())?;
    db.migrate().await.map_err(|e| e.to_string())?;
    println!("migrations applied ({})", config.db_path().display());
    Ok(())
}

async fn run_serve(config_override: Option<PathBuf>) -> Result<(), String> {
    let config = match config_override {
        Some(path) => AppConfig::from_file(&path).map_err(|e| e.to_string())?,
        None => AppConfig::load().map_err(|e| e.to_string())?,
    };

    let db = Database::open(config.db_path().to_str().ok_or("invalid db path")?)
        .await
        .map_err(|e| e.to_string())?;
    db.migrate().await.map_err(|e| e.to_string())?;

    let token = agpeer_core::token::ensure_token(std::path::Path::new(&config.data_dir))
        .map_err(|e| e.to_string())?;

    // Runtime-settable queue settings persisted via the settings API. Read them
    // before the DB is moved into AppState so backends can apply overrides when
    // they are constructed. Values are bytes/sec numbers (the web UI stores the
    // user-entered KiB/s * 1024).
    let settings = agpeer_storage::SettingsStore::new(&db)
        .all()
        .await
        .map_err(|e| e.to_string())?;
    let rate_override = |key: &str, cur: Option<u64>| -> Option<u64> {
        settings.get(key).and_then(|v| v.as_u64()).or(cur)
    };

    // Seed the magnet-search controls from config so the WebUI has stable
    // defaults. The settings live in the `settings` table and are edited at
    // runtime via the settings API; the static config values are only the
    // initial defaults.
    {
        let store = agpeer_storage::SettingsStore::new(&db);
        if !settings.contains_key("hook_search.enabled") {
            let _ = store
                .set(
                    "hook_search.enabled",
                    &serde_json::json!(config.hook_search.enabled),
                )
                .await;
        }
        if !settings.contains_key("hook_search.domains") {
            let _ = store
                .set(
                    "hook_search.domains",
                    &serde_json::json!(config.hook_search.domains),
                )
                .await;
        }
        if !settings.contains_key("hook_search.sites") {
            let _ = store
                .set(
                    "hook_search.sites",
                    &serde_json::json!(config.hook_search.sites),
                )
                .await;
        }
    }

    let state = AppState::new(config.clone(), db, token);

    if config.torrent.enabled {
        let tcfg = agpeer_torrent::TorrentConfig {
            download_root: config.torrent.download_root.clone(),
            listen_port: config.torrent.listen_port,
            download_rate_limit: rate_override(
                "queue.download_rate_limit",
                config.torrent.download_rate_limit,
            ),
            upload_rate_limit: rate_override(
                "queue.upload_rate_limit",
                config.torrent.upload_rate_limit,
            ),
            enable_dht: config.torrent.enable_dht,
            enable_pex: config.torrent.enable_pex,
            enable_lsd: config.torrent.enable_lsd,
            enable_tracker: config.torrent.enable_tracker,
        };
        let use_rqbit = config.torrent.engine.eq_ignore_ascii_case("rqbit");
        #[cfg(feature = "rqbit")]
        let engine_result = agpeer_torrent::TorrentBackend::new_rqbit(tcfg).await;
        #[cfg(not(feature = "rqbit"))]
        let engine_result = {
            if use_rqbit {
                tracing::warn!(
                    "torrent.engine = \"rqbit\" requested but the binary was built without the \
                     rqbit feature; falling back to the memory engine"
                );
            }
            agpeer_torrent::TorrentBackend::new(tcfg).await
        };
        match engine_result {
            Ok(b) => {
                tracing::info!("torrent backend registered (engine: {})", b.engine_name());
                state.register_transfer_backend(Backend::Torrent, Arc::new(b));
            }
            Err(e) => {
                tracing::error!("torrent backend failed to start: {e}");
                state.bus.publish(
                    "backend.degraded",
                    serde_json::json!({"backend": "torrent", "error": e.to_string()}),
                );
            }
        }
        let _ = use_rqbit;
    } else {
        tracing::warn!("torrent backend disabled");
    }

    if config.soulseek.enabled {
        match config.soulseek.username.as_deref() {
            Some(username) if !username.is_empty() => {
                let native_config = agpeer_soulseek::NativeConfig {
                    server_addr: config.soulseek.server_addr.clone(),
                    username: username.to_string(),
                    password: config.soulseek.password.clone().unwrap_or_default(),
                    listen_port: config.soulseek.listen_port,
                    download_dir: config.soulseek.download_root.clone(),
                    ..agpeer_soulseek::NativeConfig::default()
                };
                match agpeer_soulseek::NativeSoulseekBackend::connect(native_config).await {
                    Ok(backend) => {
                        let backend = Arc::new(backend);
                        state.register_transfer_backend(Backend::Soulseek, backend.clone());
                        state.register_search_backend(Backend::Soulseek, backend);
                        tracing::info!("soulseek backend registered (native client)");
                    }
                    Err(e) => {
                        tracing::warn!("native soulseek backend failed to connect: {e}");
                        state.bus.publish(
                            "backend.degraded",
                            serde_json::json!({"backend": "soulseek", "error": e.to_string()}),
                        );
                    }
                }
            }
            _ => {
                tracing::warn!(
                    "soulseek enabled but no soulseek.username configured; native backend \
                     unavailable"
                );
            }
        }
    } else {
        tracing::warn!("soulseek backend disabled");
    }

    // Magnet search backend, always registered. Discovery is search-only:
    // found magnets are pulled through the torrent backend, so no transfer
    // backend is registered here. With no `[hook_search].command` configured
    // the built-in domain-neutral engine/site-template search is used (zero
    // external files); a configured `command` overrides it with an external
    // script. The runtime `enabled` toggle (settings table, WebUI) decides
    // whether searches are actually permitted, so it can be flipped without a
    // restart.
    {
        let timeout = std::time::Duration::from_secs(config.hook_search.timeout_secs.max(1));
        let backend = Arc::new(agpeer_hook::HookSearchBackend::new(
            config.hook_search.command.clone(),
            timeout,
            config.hook_search.max_results,
            Some(state.db.clone()),
        ));
        state.register_search_backend(Backend::Hook, backend);
        tracing::info!(
            command_configured = !config.hook_search.command.is_empty(),
            "hook search backend registered (built-in search, runtime-enabled via settings)"
        );
    }

    spawn_ttl_sweeper(state.clone(), std::time::Duration::from_secs(60));
    spawn_transfer_sync(state.clone(), std::time::Duration::from_secs(2));

    let app = router(state.clone());
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("failed to bind {addr}: {e}"))?;
    tracing::info!("agpeer core listening on http://{addr}");
    tracing::info!("API docs: http://{addr}/api/v1/docs");

    let serve_result = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .map_err(|e| format!("server error: {e}"));

    serve_result
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install ctrl-c handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parses_migrate() {
        let cli = Cli::try_parse_from(["agpeer", "migrate"]).expect("migrate should parse");
        assert!(matches!(cli.command, Command::Migrate { .. }));
    }

    #[test]
    fn parses_serve() {
        let cli = Cli::try_parse_from(["agpeer", "serve"]).expect("serve should parse");
        assert!(matches!(cli.command, Command::Serve { .. }));
    }

    #[test]
    fn rotates_at_line_limit_and_keeps_bounded_retention() {
        let dir = std::env::temp_dir().join(format!(
            "agpeer-log-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos()
        ));
        let active = dir.join("agpeer.log");
        let mut writer = LineRotatingWriter::new(&dir).expect("writer should open");

        for _ in 0..(LOG_LINES_PER_FILE * (LOG_FILE_RETENTION + 1)) {
            writer
                .write_all(b"line\n")
                .expect("log write should succeed");
        }
        writer.flush().expect("log flush should succeed");

        let files: Vec<_> = std::fs::read_dir(&dir)
            .expect("log directory should be readable")
            .filter_map(Result::ok)
            .collect();
        assert_eq!(files.len(), LOG_FILE_RETENTION);
        assert_eq!(std::fs::read_to_string(&active).unwrap(), "");
        assert_eq!(
            count_lines(&rotated_path(&active, 1)).unwrap(),
            LOG_LINES_PER_FILE
        );

        drop(writer);
        std::fs::remove_dir_all(dir).expect("test log directory should be removable");
    }
}
