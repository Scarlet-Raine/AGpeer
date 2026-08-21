//! Configuration for the torrent backend.

use serde::{Deserialize, Serialize};

/// Static configuration for the torrent transfer backend.
///
/// `download_root` is the default destination directory used when an
/// `AddTransferRequest` does not specify one. Every field is serializable so
/// the value can be persisted as part of the application configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TorrentConfig {
    /// Default destination directory for transfers without an explicit one.
    pub download_root: String,
    /// Port for the BitTorrent listen socket. `None` lets the backend pick a
    /// free port (or, for the in-memory engine, disables listening entirely).
    pub listen_port: Option<u16>,
    /// Global download rate limit in bytes per second.
    pub download_rate_limit: Option<u64>,
    /// Global upload rate limit in bytes per second.
    pub upload_rate_limit: Option<u64>,
    /// Whether to enable the Distributed Hash Table for peer discovery.
    pub enable_dht: bool,
    /// Whether to enable Peer EXchange. Not all backends can toggle this;
    /// see SPIKE.md.
    pub enable_pex: bool,
    /// Whether to enable Local Service Discovery (multicast peer discovery).
    pub enable_lsd: bool,
    /// Whether to enable tracker announcements.
    pub enable_tracker: bool,
}

impl Default for TorrentConfig {
    fn default() -> Self {
        Self {
            download_root: std::env::temp_dir()
                .join("agpeer-downloads")
                .to_string_lossy()
                .into_owned(),
            listen_port: None,
            download_rate_limit: None,
            upload_rate_limit: None,
            enable_dht: true,
            enable_pex: true,
            enable_lsd: false,
            enable_tracker: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let cfg = TorrentConfig::default();
        assert!(!cfg.download_root.is_empty());
        assert!(cfg.enable_dht);
        assert!(cfg.enable_pex);
        assert!(cfg.enable_tracker);
        assert!(!cfg.enable_lsd);
        assert!(cfg.listen_port.is_none());
        assert!(cfg.download_rate_limit.is_none());
    }

    #[test]
    fn serde_roundtrip() {
        let cfg = TorrentConfig {
            download_root: "C:\\downloads".into(),
            listen_port: Some(42000),
            download_rate_limit: Some(1024 * 1024),
            upload_rate_limit: None,
            enable_dht: false,
            enable_pex: false,
            enable_lsd: true,
            enable_tracker: false,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: TorrentConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, back);
    }
}
