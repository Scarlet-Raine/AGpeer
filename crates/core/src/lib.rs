//! agpeer core: configuration, secrets, event bus, application state, and
//! housekeeping (search TTL expiry, transfer reconciliation).

pub mod config;
pub mod event;
pub mod housekeeping;
pub mod postprocess;
pub mod secrets;
pub mod state;
pub mod token;

pub use config::{
    default_config_path, default_data_dir, AppConfig, ConfigError, PostprocessConfig, ServerConfig,
    SoulseekConfig, TorrentConfig,
};
pub use event::{AppEvent, EventBus};
pub use secrets::{FileSecretStore, KeyringSecretStore, MemorySecretStore, SecretStore};
pub use state::AppState;
pub use token::ensure_token;
