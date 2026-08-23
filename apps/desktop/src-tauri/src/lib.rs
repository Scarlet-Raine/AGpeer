//! agpeer desktop shell (Tauri 2).
//!
//! Responsibilities:
//! - expose a `get_api_token` command so the frontend can authenticate against
//!   the agpeer core service;
//! - optionally spawn the `agpeer` core binary as a child process when the
//!   `AGPEER_CORE_BIN` environment variable is set.

/// Read the core API token.
///
/// Resolution order:
/// 1. `AGPEER_TOKEN_FILE` environment variable (exact token file);
/// 2. `AGPEER_DATA_DIR` environment variable → `<dir>/token` (the directory
///    the core persists its token into);
/// 3. repo-local dev fallback `run/data/token` (what the core writes when run
///    from this workspace with `run/config.toml`);
/// 4. `<OS app-data dir>/agpeer/data/token` — mirrors the core's
///    `ProjectDirs::from("dev", "agpeer", "agpeer").data_dir()` default
///    (`%APPDATA%\agpeer\data` on Windows, `~/.local/share/agpeer` on Linux).
#[tauri::command]
fn get_api_token() -> Result<String, String> {
    if let Ok(path) = std::env::var("AGPEER_TOKEN_FILE") {
        if let Ok(raw) = std::fs::read_to_string(&path) {
            let t = raw.trim();
            if !t.is_empty() {
                return Ok(t.to_string());
            }
        }
    }
    if let Ok(dir) = std::env::var("AGPEER_DATA_DIR") {
        let p = std::path::Path::new(&dir).join("token");
        if let Ok(raw) = std::fs::read_to_string(&p) {
            let t = raw.trim();
            if !t.is_empty() {
                return Ok(t.to_string());
            }
        }
    }
    let cwd_token = std::env::current_dir()
        .map(|c| c.join("run").join("data").join("token"))
        .ok();
    if let Some(p) = cwd_token {
        if let Ok(raw) = std::fs::read_to_string(&p) {
            let t = raw.trim();
            if !t.is_empty() {
                return Ok(t.to_string());
            }
        }
    }
    let token_path = directories::ProjectDirs::from("dev", "agpeer", "agpeer")
        .map(|dirs| dirs.data_dir().join("token"))
        .ok_or_else(|| "no OS app-data directory available".to_string())?;
    let raw = std::fs::read_to_string(&token_path)
        .map_err(|e| format!("no token at {}: {e}", token_path.display()))?;
    let t = raw.trim();
    if t.is_empty() {
        return Err("token file is empty".into());
    }
    Ok(t.to_string())
}

/// Open a local folder in the operating system's file manager (Explorer on
/// Windows). Used by the Library screen's "open folder" action.
#[tauri::command]
fn open_path(path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("failed to open {}: {e}", path))?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("failed to open {}: {e}", path))?;
    }
    Ok(())
}

/// Optionally spawn the core service as a child process.
///
/// Set `AGPEER_CORE_BIN` to the path of the `agpeer` binary to have the
/// desktop shell start the core on launch (the core is then stopped when the
/// shell exits). When unset, the app expects a core already running.
fn spawn_core_if_requested() {
    let Some(bin) = std::env::var_os("AGPEER_CORE_BIN") else {
        return;
    };
    match std::process::Command::new(&bin).arg("serve").spawn() {
        Ok(_) => {
            tracing::info!("spawned agpeer core: {}", bin.to_string_lossy());
        }
        Err(e) => {
            eprintln!("failed to spawn agpeer core ({}): {e}", bin.to_string_lossy());
        }
    }
}

/// Run the Tauri application.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|_app| {
            spawn_core_if_requested();
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_api_token, open_path])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
