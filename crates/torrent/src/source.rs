//! Source validation and normalization shared by every engine.
//!
//! Every accepted torrent source is one of:
//!
//! - a magnet URI (must start with `magnet:?`),
//! - a path to an existing local `.torrent` file,
//! - a remote `.torrent` URL (`http://` or `https://`).
//!
//! Anything else is rejected with [`BackendError::InvalidSource`].

use std::path::{Path, PathBuf};

use crate::error::BackendError;
use agpeer_common::percent_decode;

/// A validated torrent source.
#[derive(Debug, Clone)]
pub(crate) enum SourceKind {
    /// A magnet URI. `info_hash` and `display_name` are the `xt`/`dn` query
    /// values when present.
    Magnet {
        info_hash: Option<String>,
        display_name: Option<String>,
    },
    /// A path to an existing local `.torrent` file.
    TorrentFile(PathBuf),
    /// A remote `.torrent` URL. The reference engine does not fetch it; the
    /// real engine passes it to librqbit which downloads and parses it.
    TorrentUrl(String),
}

/// Validate `source` and classify it.
pub(crate) fn parse_source(source: &str) -> Result<SourceKind, BackendError> {
    if source.starts_with("magnet:?") {
        return Ok(SourceKind::Magnet {
            info_hash: magnet_query_value(source, "xt"),
            display_name: magnet_query_value(source, "dn").map(|dn| percent_decode(&dn)),
        });
    }

    let lower = source.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Ok(SourceKind::TorrentUrl(source.to_string()));
    }

    let path = Path::new(source);
    if path.is_file() {
        return Ok(SourceKind::TorrentFile(path.to_path_buf()));
    }

    Err(BackendError::InvalidSource)
}

/// A sensible display name derived from the source when the caller supplied
/// none and the engine has no parsed metainfo yet.
pub(crate) fn fallback_name(kind: &SourceKind) -> String {
    match kind {
        SourceKind::Magnet {
            display_name,
            info_hash,
            ..
        } => display_name
            .clone()
            .or_else(|| info_hash.as_ref().map(|h| format!("magnet-{h}")))
            .unwrap_or_else(|| "magnet".to_string()),
        SourceKind::TorrentFile(path) => path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_string)
            .unwrap_or_else(|| "torrent".to_string()),
        SourceKind::TorrentUrl(url) => url
            .split('/')
            .rfind(|s| !s.is_empty())
            .unwrap_or("download")
            .to_string(),
    }
}

/// Extract the value of `key` from the query part of a magnet URI. The
/// `urn:btih:` prefix is stripped from the `xt` value.
fn magnet_query_value(magnet: &str, key: &str) -> Option<String> {
    let query = magnet.split_once('?').map(|(_, q)| q).unwrap_or(magnet);
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=')?;
        if k == key {
            let v = v.to_string();
            let v = if key == "xt" {
                v.strip_prefix("urn:btih:").map(str::to_string).unwrap_or(v)
            } else {
                v
            };
            return Some(v);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAGNET: &str =
        "magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862&dn=Ubuntu+22.04";

    #[test]
    fn valid_magnet_is_accepted() {
        match parse_source(MAGNET).unwrap() {
            SourceKind::Magnet {
                info_hash,
                display_name,
                ..
            } => {
                assert_eq!(
                    info_hash.as_deref(),
                    Some("cab507494d02ebb1178b38f2e9d7be299c86b862")
                );
                assert_eq!(display_name.as_deref(), Some("Ubuntu 22.04"));
            }
            other => panic!("expected magnet, got {other:?}"),
        }
    }

    #[test]
    fn magnet_without_question_mark_is_rejected() {
        assert!(matches!(
            parse_source("magnet:xt=urn:btih:deadbeef"),
            Err(BackendError::InvalidSource)
        ));
    }

    #[test]
    fn magnet_without_xt_still_parses() {
        match parse_source("magnet:?dn=NoHash").unwrap() {
            SourceKind::Magnet {
                info_hash,
                display_name,
                ..
            } => {
                assert!(info_hash.is_none());
                assert_eq!(display_name.as_deref(), Some("NoHash"));
            }
            other => panic!("expected magnet, got {other:?}"),
        }
    }

    #[test]
    fn remote_urls_are_accepted() {
        assert!(matches!(
            parse_source("https://example.com/x.torrent").unwrap(),
            SourceKind::TorrentUrl(_)
        ));
        assert!(matches!(
            parse_source("HTTP://example.com/x.torrent").unwrap(),
            SourceKind::TorrentUrl(_)
        ));
        assert!(matches!(
            parse_source("ftp://example.com/x.torrent"),
            Err(BackendError::InvalidSource)
        ));
    }

    #[test]
    fn existing_file_path_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.torrent");
        std::fs::write(&path, b"not really a torrent").unwrap();
        assert!(matches!(
            parse_source(path.to_str().unwrap()).unwrap(),
            SourceKind::TorrentFile(_)
        ));
    }

    #[test]
    fn missing_file_is_rejected() {
        let missing = std::env::temp_dir().join("agpeer-does-not-exist.torrent");
        assert!(matches!(
            parse_source(missing.to_str().unwrap()),
            Err(BackendError::InvalidSource)
        ));
    }

    #[test]
    fn garbage_is_rejected() {
        for bad in ["", "  ", "wat", "C:\\", "magnet"] {
            assert!(
                matches!(parse_source(bad), Err(BackendError::InvalidSource)),
                "expected {bad:?} to be rejected"
            );
        }
    }

    #[test]
    fn fallback_names() {
        assert_eq!(
            fallback_name(&parse_source(MAGNET).unwrap()),
            "Ubuntu 22.04"
        );
        assert_eq!(
            fallback_name(&parse_source("https://x.com/a/b.torrent").unwrap()),
            "b.torrent"
        );
    }
}
