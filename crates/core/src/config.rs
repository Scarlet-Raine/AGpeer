//! Application configuration.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 41000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TorrentConfig {
    pub enabled: bool,
    /// Which engine to use: `"memory"` (reference/dev) or `"rqbit"` (real,
    /// requires the binary to be built with the `rqbit` feature).
    pub engine: String,
    pub download_root: String,
    pub listen_port: Option<u16>,
    pub download_rate_limit: Option<u64>,
    pub upload_rate_limit: Option<u64>,
    pub enable_dht: bool,
    pub enable_pex: bool,
    pub enable_lsd: bool,
    pub enable_tracker: bool,
}

impl Default for TorrentConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            engine: "memory".to_string(),
            download_root: "downloads".to_string(),
            listen_port: None,
            download_rate_limit: None,
            upload_rate_limit: None,
            enable_dht: true,
            enable_pex: true,
            enable_lsd: true,
            enable_tracker: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SoulseekConfig {
    pub enabled: bool,
    /// Soulseek server address.
    pub server_addr: String,
    /// Listen port announced for peer/file connections.
    pub listen_port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub download_root: String,
}

impl Default for SoulseekConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            server_addr: "vps.slsknet.org:2242".to_string(),
            listen_port: 2234,
            username: None,
            password: None,
            download_root: "downloads".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HookSearchConfig {
    pub enabled: bool,
    /// The external search command to run per query. Arg 0 is the program; the
    /// literal token `{query}` is replaced with the search term and `{domains}`
    /// with the comma-joined domain list (query/domains are appended as final
    /// arguments when not referenced). No shell is used; values pass verbatim.
    /// Empty = built-in search (generic engine + site templates).
    pub command: Vec<String>,
    /// Initial domain list handed to the built-in engine search (editable at
    /// runtime in the WebUI via the `hook_search.domains` setting).
    pub domains: Vec<String>,
    /// Initial per-site search templates (editable at runtime via the
    /// `hook_search.sites` setting). User-configured only; the binary compiles
    /// no site behavior.
    pub sites: Vec<agpeer_common::HookSearchSite>,
    pub timeout_secs: u64,
    pub max_results: usize,
}

impl Default for HookSearchConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            command: Vec::new(),
            domains: Vec::new(),
            sites: Vec::new(),
            timeout_secs: 30,
            max_results: 100,
        }
    }
}

pub fn default_data_dir() -> PathBuf {
    directories::ProjectDirs::from("dev", "agpeer", "agpeer")
        .map(|dirs| dirs.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("data"))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PostprocessConfig {
    /// Library root for organized media, e.g. `E:\Media`. Empty disables
    /// automatic organization.
    pub library_root: String,
    /// Move completed downloads into the library tree automatically.
    pub auto_organize: bool,
    /// Subfolder under `library_root` for series episodes
    /// (default `TV Shows`).
    pub tv_dir: Option<String>,
    /// Subfolder under `library_root` for films (default `Movies`).
    pub movies_dir: Option<String>,
    /// Subfolder under `library_root` for anime episodes; when unset, anime
    /// goes to `tv_dir`.
    pub anime_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub data_dir: String,
    pub torrent: TorrentConfig,
    pub soulseek: SoulseekConfig,
    pub hook_search: HookSearchConfig,
    pub postprocess: PostprocessConfig,
    pub search_result_ttl_hours: u64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            data_dir: default_data_dir().to_string_lossy().into_owned(),
            torrent: TorrentConfig::default(),
            soulseek: SoulseekConfig::default(),
            hook_search: HookSearchConfig::default(),
            postprocess: PostprocessConfig::default(),
            search_result_ttl_hours: 24,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config io error: {0}")]
    Io(String),
    #[error("config parse error: {0}")]
    Toml(String),
}

pub fn default_config_path() -> PathBuf {
    directories::ProjectDirs::from("dev", "agpeer", "agpeer")
        .map(|dirs| dirs.config_dir().join("agpeer.toml"))
        .unwrap_or_else(|| PathBuf::from("agpeer.toml"))
}

impl AppConfig {
    pub fn ensure_dirs(&self) -> Result<(), ConfigError> {
        std::fs::create_dir_all(&self.data_dir).map_err(|e| ConfigError::Io(e.to_string()))?;
        std::fs::create_dir_all(&self.torrent.download_root)
            .map_err(|e| ConfigError::Io(e.to_string()))?;
        Ok(())
    }

    /// Overlay runtime values from environment variables. Currently this lets
    /// connection/soulseek settings be supplied per-run without editing the
    /// config file (or committing credentials):
    ///
    /// - `AGPEER_HOST` — bind host for the HTTP server
    /// - `AGPEER_PORT` — HTTP server port
    /// - `AGPEER_DATA_DIR` — data directory (DB, token, logs)
    /// - `AGPEER_SOULSEEK_SERVER_ADDR` — Soulseek server address
    /// - `AGPEER_SOULSEEK_LISTEN_PORT` — local listen port for peer/file connections
    /// - `AGPEER_SOULSEEK_USERNAME`, `AGPEER_SOULSEEK_PASSWORD`
    /// - `AGPEER_TORRENT_LISTEN_PORT` — inbound BitTorrent peer port
    /// - `AGPEER_TORRENT_DOWNLOAD_ROOT`, `AGPEER_SOULSEEK_DOWNLOAD_ROOT`,
    ///   `AGPEER_POSTPROCESS_LIBRARY_ROOT` — storage locations (container
    ///   deployments can run entirely from env, no config file)
    /// - `AGPEER_TORRENT_ENABLED`, `AGPEER_TORRENT_ENGINE`,
    ///   `AGPEER_SOULSEEK_ENABLED`, `AGPEER_HOOK_SEARCH_ENABLED` — backend
    ///   toggles (defaults are conservative; container images enable them)
    pub fn apply_env_overrides(&mut self) {
        self.apply_env_overrides_from(|key| std::env::var(key).ok());
    }

    /// Same as [`Self::apply_env_overrides`] but reads values from a caller-
    /// supplied lookup, which keeps the logic testable without mutating the
    /// process-global environment.
    pub fn apply_env_overrides_from(&mut self, get: impl Fn(&str) -> Option<String>) {
        if let Some(host) = get("AGPEER_HOST") {
            if !host.trim().is_empty() {
                self.server.host = host;
            }
        }
        if let Some(port) = get("AGPEER_PORT") {
            if let Ok(port) = port.parse() {
                self.server.port = port;
            }
        }
        if let Some(dir) = get("AGPEER_DATA_DIR") {
            if !dir.trim().is_empty() {
                self.data_dir = dir;
            }
        }
        if let Some(addr) = get("AGPEER_SOULSEEK_SERVER_ADDR") {
            if !addr.trim().is_empty() {
                self.soulseek.server_addr = addr;
            }
        }
        if let Some(root) = get("AGPEER_TORRENT_DOWNLOAD_ROOT") {
            if !root.trim().is_empty() {
                self.torrent.download_root = root;
            }
        }
        if let Some(root) = get("AGPEER_SOULSEEK_DOWNLOAD_ROOT") {
            if !root.trim().is_empty() {
                self.soulseek.download_root = root;
            }
        }
        if let Some(root) = get("AGPEER_POSTPROCESS_LIBRARY_ROOT") {
            if !root.trim().is_empty() {
                self.postprocess.library_root = root;
            }
        }
        if let Some(enabled) = get("AGPEER_TORRENT_ENABLED") {
            if let Ok(enabled) = enabled.trim().parse() {
                self.torrent.enabled = enabled;
            }
        }
        if let Some(engine) = get("AGPEER_TORRENT_ENGINE") {
            if !engine.trim().is_empty() {
                self.torrent.engine = engine;
            }
        }
        if let Some(enabled) = get("AGPEER_SOULSEEK_ENABLED") {
            if let Ok(enabled) = enabled.trim().parse() {
                self.soulseek.enabled = enabled;
            }
        }
        if let Some(enabled) = get("AGPEER_HOOK_SEARCH_ENABLED") {
            if let Ok(enabled) = enabled.trim().parse() {
                self.hook_search.enabled = enabled;
            }
        }
        if let Some(port) = get("AGPEER_SOULSEEK_LISTEN_PORT") {
            if let Ok(port) = port.parse() {
                self.soulseek.listen_port = port;
            }
        }
        if let Some(port) = get("AGPEER_TORRENT_LISTEN_PORT") {
            if let Ok(port) = port.parse() {
                self.torrent.listen_port = Some(port);
            }
        }
        if let Some(username) = get("AGPEER_SOULSEEK_USERNAME") {
            self.soulseek.username = Some(username);
        }
        if let Some(password) = get("AGPEER_SOULSEEK_PASSWORD") {
            self.soulseek.password = Some(password);
        }
    }

    pub fn from_file(path: &Path) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::Io(format!("{}: {}", path.display(), e)))?;
        let mut config: Self = toml::from_str(&contents)
            .map_err(|e| ConfigError::Toml(format!("{}: {}", path.display(), e)))?;
        // A missing or misplaced (e.g. table-scoped) `data_dir` key falls back
        // to the per-user default directory instead of an empty string.
        if config.data_dir.trim().is_empty() {
            config.data_dir = default_data_dir().to_string_lossy().into_owned();
        }
        config.apply_env_overrides();
        config.ensure_dirs()?;
        Ok(config)
    }

    pub fn load() -> Result<Self, ConfigError> {
        let path = std::env::var_os("AGPEER_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(default_config_path);
        if path.exists() {
            Self::from_file(&path)
        } else {
            let mut config = Self::default();
            config.apply_env_overrides();
            config.ensure_dirs()?;
            let _ = std::fs::write(&path, toml::to_string(&config).unwrap_or_default());
            Ok(config)
        }
    }

    pub fn db_path(&self) -> PathBuf {
        Path::new(&self.data_dir).join("agpeer.sqlite")
    }

    pub fn token_file(&self) -> PathBuf {
        Path::new(&self.data_dir).join("token")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_use_loopback_port() {
        let config = AppConfig::default();
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 41000);
    }

    #[test]
    fn from_file_roundtrip_applies_defaults() {
        let dir = std::env::temp_dir().join(format!("agpeer-config-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        // Single-quoted TOML literal so Windows backslashes are not treated as
        // escapes; data_dir is a top-level key, not part of [server].
        std::fs::write(
            &path,
            format!(
                "[server]\nhost = \"10.0.0.1\"\nport = 5000\n\ndata_dir = '{}'\n",
                dir.display()
            ),
        )
        .unwrap();

        let config = AppConfig::from_file(&path).unwrap();
        assert_eq!(config.server.host, "10.0.0.1");
        assert_eq!(config.server.port, 5000);
        assert_eq!(config.search_result_ttl_hours, 24);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn db_path_joins_data_dir() {
        let config = AppConfig {
            data_dir: "test_data_dir".to_string(),
            ..AppConfig::default()
        };
        assert_eq!(
            config.db_path(),
            PathBuf::from("test_data_dir/agpeer.sqlite")
        );
        assert_eq!(config.token_file(), PathBuf::from("test_data_dir/token"));
    }

    #[test]
    fn soulseek_env_overrides_apply() {
        let mut config = AppConfig::default();
        let overrides = [
            ("AGPEER_SOULSEEK_SERVER_ADDR", "srv.example:1234"),
            ("AGPEER_SOULSEEK_LISTEN_PORT", "4321"),
            ("AGPEER_SOULSEEK_USERNAME", "alice"),
            ("AGPEER_SOULSEEK_PASSWORD", "pw"),
        ]
        .into_iter()
        .collect::<std::collections::HashMap<_, _>>();
        config.apply_env_overrides_from(|key| overrides.get(key).map(|v| v.to_string()));

        assert_eq!(config.soulseek.server_addr, "srv.example:1234");
        assert_eq!(config.soulseek.listen_port, 4321);
        assert_eq!(config.soulseek.username.as_deref(), Some("alice"));
        assert_eq!(config.soulseek.password.as_deref(), Some("pw"));
    }

    #[test]
    fn storage_root_env_overrides_apply() {
        let mut config = AppConfig::default();
        let overrides = [
            ("AGPEER_TORRENT_DOWNLOAD_ROOT", "/opt/agpeer/downloads"),
            (
                "AGPEER_SOULSEEK_DOWNLOAD_ROOT",
                "/opt/agpeer/soulseek-downloads",
            ),
            ("AGPEER_POSTPROCESS_LIBRARY_ROOT", "/opt/agpeer/library"),
            ("AGPEER_TORRENT_LISTEN_PORT", "51234"),
            ("AGPEER_TORRENT_ENABLED", "true"),
            ("AGPEER_TORRENT_ENGINE", "rqbit"),
            ("AGPEER_SOULSEEK_ENABLED", "true"),
            ("AGPEER_HOOK_SEARCH_ENABLED", "true"),
        ]
        .into_iter()
        .collect::<std::collections::HashMap<_, _>>();
        config.apply_env_overrides_from(|key| overrides.get(key).map(|v| v.to_string()));

        assert_eq!(config.torrent.download_root, "/opt/agpeer/downloads");
        assert_eq!(
            config.soulseek.download_root,
            "/opt/agpeer/soulseek-downloads"
        );
        assert_eq!(config.postprocess.library_root, "/opt/agpeer/library");
        assert_eq!(config.torrent.listen_port, Some(51234));
        assert!(config.torrent.enabled);
        assert_eq!(config.torrent.engine, "rqbit");
        assert!(config.soulseek.enabled);
        assert!(config.hook_search.enabled);
    }

    #[test]
    fn storage_root_env_overrides_ignore_empty_values() {
        let mut config = AppConfig::default();
        let torrent_root = config.torrent.download_root.clone();
        let torrent_enabled = config.torrent.enabled;
        let overrides = std::collections::HashMap::from([
            ("AGPEER_TORRENT_DOWNLOAD_ROOT", "  "),
            ("AGPEER_TORRENT_ENABLED", "not-a-bool"),
        ]);
        config.apply_env_overrides_from(|key| overrides.get(key).map(|v| v.to_string()));
        assert_eq!(config.torrent.download_root, torrent_root);
        assert_eq!(config.torrent.enabled, torrent_enabled);
    }

    #[test]
    fn soulseek_env_override_ignores_invalid_port() {
        let mut config = AppConfig::default();
        let port = config.soulseek.listen_port;
        let overrides =
            std::collections::HashMap::from([("AGPEER_SOULSEEK_LISTEN_PORT", "not-a-port")]);
        config.apply_env_overrides_from(|key| overrides.get(key).map(|v| v.to_string()));
        assert_eq!(config.soulseek.listen_port, port);
    }

    #[test]
    fn runtime_env_override_host_port_data_dir() {
        let mut config = AppConfig::default();
        let overrides = [
            ("AGPEER_HOST", "0.0.0.0"),
            ("AGPEER_PORT", "42000"),
            ("AGPEER_DATA_DIR", "C:/tmp/agpeer-env-test"),
        ]
        .into_iter()
        .collect::<std::collections::HashMap<_, _>>();
        config.apply_env_overrides_from(|key| overrides.get(key).map(|v| v.to_string()));

        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.server.port, 42000);
        assert_eq!(config.data_dir, "C:/tmp/agpeer-env-test");
    }

    #[test]
    fn runtime_env_override_ignores_invalid_port() {
        let mut config = AppConfig::default();
        let default_port = config.server.port;
        let overrides = std::collections::HashMap::from([("AGPEER_PORT", "not-a-port")]);
        config.apply_env_overrides_from(|key| overrides.get(key).map(|v| v.to_string()));
        assert_eq!(config.server.port, default_port);
    }

    #[test]
    fn env_overrides_apply_before_serve_binding() {
        // A headless "env-only" launch (no TOML edits) must be able to move the
        // bind address, port, and data dir without a config file.
        let mut config = AppConfig::default();
        config.apply_env_overrides_from(|key| match key {
            "AGPEER_HOST" => Some("0.0.0.0".into()),
            "AGPEER_DATA_DIR" => Some("temp-dir".into()),
            _ => None,
        });
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.data_dir, "temp-dir");
    }

    #[test]
    fn soulseek_env_missing_values_keep_defaults() {
        let config = AppConfig::default();
        let default_addr = config.soulseek.server_addr.clone();
        let mut config = AppConfig::default();
        config.apply_env_overrides_from(|_| None);
        assert_eq!(config.soulseek.server_addr, default_addr);
        assert!(config.soulseek.username.is_none());
    }
}
