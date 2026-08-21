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
    pub command: Vec<String>,
    /// Initial domain list handed to the hook command (editable at runtime in
    /// the WebUI via the `hook_search.domains` setting).
    pub domains: Vec<String>,
    pub timeout_secs: u64,
    pub max_results: usize,
}

impl Default for HookSearchConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            command: Vec::new(),
            domains: Vec::new(),
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
    /// Soulseek credentials be supplied per-run without editing the config file
    /// (or committing them):
    ///
    /// - `AGPEER_SOULSEEK_USERNAME`, `AGPEER_SOULSEEK_PASSWORD`
    pub fn apply_env_overrides(&mut self) {
        if let Ok(username) = std::env::var("AGPEER_SOULSEEK_USERNAME") {
            self.soulseek.username = Some(username);
        }
        if let Ok(password) = std::env::var("AGPEER_SOULSEEK_PASSWORD") {
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
}
