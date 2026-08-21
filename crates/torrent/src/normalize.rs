//! Normalization helpers shared by the engines: destination resolution,
//! file-selection application, and namespaced backend metadata.

use std::collections::HashMap;
use std::path::Path;

use agpeer_common::{AddTransferRequest, FileSelection, Transfer, TransferFile};
use serde_json::{Map, Value};

use crate::config::TorrentConfig;
use crate::error::BackendError;

/// Resolve the destination directory for a request.
///
/// A caller-supplied destination must be non-empty and absolute; otherwise the
/// configured download root is used.
pub(crate) fn resolve_destination(
    request: &AddTransferRequest,
    config: &TorrentConfig,
) -> Result<String, BackendError> {
    match request.destination.as_deref() {
        Some(destination) => {
            Transfer::validate_destination(destination).map_err(|_| BackendError::InvalidSource)?;
            if !Path::new(destination).is_absolute() {
                return Err(BackendError::UnsafePath);
            }
            Ok(destination.to_string())
        }
        None => Ok(config.download_root.clone()),
    }
}

/// Apply the request's per-file selection to the torrent's file list.
///
/// Selection entries whose index does not match any file are ignored, which
/// keeps magnet-sourced transfers (whose file list is unknown until metadata
/// resolves) working.
pub(crate) fn apply_file_selection(
    files: &mut [TransferFile],
    selection: Option<&[FileSelection]>,
) {
    let Some(selection) = selection else {
        return;
    };
    for item in selection {
        if let Some(file) = files.iter_mut().find(|f| f.index == item.index) {
            file.selected = item.selected;
        }
    }
}

/// Build the `"torrent"` metadata object for a transfer.
///
/// All backend-specific metadata lives under this key. `private` is only
/// recorded when the source/info declares the torrent private.
pub(crate) fn torrent_metadata(engine: &str, info_hash: Option<&str>, private: bool) -> Value {
    let mut map = Map::new();
    map.insert("engine".to_string(), Value::String(engine.to_string()));
    if let Some(hash) = info_hash {
        map.insert("info_hash".to_string(), Value::String(hash.to_string()));
    }
    if private {
        map.insert("private".to_string(), Value::Bool(true));
    }
    Value::Object(map)
}

/// Combine caller-supplied metadata with the namespaced torrent metadata.
pub(crate) fn merged_metadata(
    caller: &HashMap<String, Value>,
    torrent: Value,
) -> HashMap<String, Value> {
    let mut metadata = caller.clone();
    metadata.insert("torrent".to_string(), torrent);
    metadata
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_destination_uses_download_root() {
        let config = TorrentConfig {
            download_root: "C:\\downloads".into(),
            ..TorrentConfig::default()
        };
        let request = AddTransferRequest {
            backend: agpeer_common::Backend::Torrent,
            source: "magnet:?xt=urn:btih:abc".into(),
            destination: None,
            display_name: None,
            file_selection: None,
            metadata: HashMap::new(),
        };
        assert_eq!(
            resolve_destination(&request, &config).unwrap(),
            "C:\\downloads"
        );
    }

    #[test]
    fn relative_destination_is_rejected() {
        let config = TorrentConfig::default();
        let request = AddTransferRequest {
            backend: agpeer_common::Backend::Torrent,
            source: "magnet:?xt=urn:btih:abc".into(),
            destination: Some("relative/path".into()),
            display_name: None,
            file_selection: None,
            metadata: HashMap::new(),
        };
        assert!(matches!(
            resolve_destination(&request, &config),
            Err(BackendError::UnsafePath)
        ));
    }

    #[test]
    fn file_selection_is_applied_by_index() {
        let mut files = vec![
            TransferFile {
                index: "0".into(),
                path: "a".into(),
                size: 10,
                selected: true,
                bytes_completed: 0,
            },
            TransferFile {
                index: "1".into(),
                path: "b".into(),
                size: 20,
                selected: true,
                bytes_completed: 0,
            },
        ];
        let selection = vec![
            FileSelection {
                index: "0".into(),
                selected: true,
            },
            FileSelection {
                index: "1".into(),
                selected: false,
            },
            FileSelection {
                index: "999".into(),
                selected: true,
            },
        ];
        apply_file_selection(&mut files, Some(&selection));
        assert!(files[0].selected);
        assert!(!files[1].selected);
    }

    #[test]
    fn metadata_is_namespaced_and_private_only_when_true() {
        let m = torrent_metadata("memory", Some("abc"), true);
        assert_eq!(m["engine"], "memory");
        assert_eq!(m["info_hash"], "abc");
        assert_eq!(m["private"], true);

        let m = torrent_metadata("memory", None, false);
        assert!(m.get("private").is_none());
        assert!(m.get("info_hash").is_none());
    }
}
