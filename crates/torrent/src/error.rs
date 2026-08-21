//! Typed errors for the torrent backend.
//!
//! [`BackendError`] is the crate-internal error type. It converts losslessly
//! into [`agpeer_common::Error`] so the rest of the application only ever sees
//! the shared error vocabulary.

use thiserror::Error;

/// Errors produced by the torrent backend.
#[derive(Debug, Error)]
pub enum BackendError {
    /// The supplied source is not a magnet, existing `.torrent` file, or
    /// remote `.torrent` URL.
    #[error("invalid source")]
    InvalidSource,

    /// The requested transfer does not exist.
    #[error("transfer not found")]
    TransferNotFound,

    /// The operation conflicts with the transfer's current state.
    #[error("invalid state: {0}")]
    InvalidState(String),

    /// The backend does not support the requested operation or source.
    #[error("unsupported: {0}")]
    Unsupported(String),

    /// The transfer backend is not running.
    #[error("backend unavailable")]
    Unavailable,

    /// A path escaped a configured root or was otherwise rejected.
    #[error("unsafe path")]
    UnsafePath,

    /// An I/O failure while reading sources or writing data.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// An unexpected internal failure. Must not be surfaced verbatim to
    /// remote clients.
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<BackendError> for agpeer_common::Error {
    fn from(value: BackendError) -> Self {
        match value {
            BackendError::InvalidSource => Self::InvalidSource,
            BackendError::TransferNotFound => Self::TransferNotFound,
            BackendError::InvalidState(s) => Self::InvalidState(s),
            BackendError::Unsupported(s) => Self::Backend(s),
            BackendError::Unavailable => Self::BackendUnavailable,
            BackendError::UnsafePath => Self::UnsafePath,
            BackendError::Io(e) => Self::Backend(format!("io error: {e}")),
            BackendError::Internal(s) => Self::Internal(s),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agpeer_common::Error;

    fn code(e: BackendError) -> &'static str {
        Error::from(e).code()
    }

    #[test]
    fn variants_map_to_stable_codes() {
        assert_eq!(code(BackendError::InvalidSource), "InvalidSource");
        assert_eq!(code(BackendError::TransferNotFound), "TransferNotFound");
        assert_eq!(code(BackendError::UnsafePath), "UnsafePath");
        assert_eq!(code(BackendError::Unavailable), "BackendUnavailable");
        assert_eq!(
            code(BackendError::InvalidState("paused".into())),
            "InvalidState"
        );
        assert_eq!(code(BackendError::Unsupported("x".into())), "Backend");
        assert_eq!(
            code(BackendError::Io(std::io::Error::other("boom"))),
            "Backend"
        );
        assert_eq!(code(BackendError::Internal("x".into())), "Internal");
    }

    #[test]
    fn internal_details_are_not_leaked_to_api_clients() {
        let api = Error::from(BackendError::Internal("secret internals".into())).into_api();
        assert_eq!(api.code, "Internal");
        assert!(!api.message.contains("secret internals"));
    }
}
