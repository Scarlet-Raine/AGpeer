//! Typed application errors.
//!
//! These errors are surfaced to API clients; they must never contain raw
//! internal stack traces or secrets. Detailed diagnostics belong in local
//! logs, not in the public error response.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A machine-readable error code plus a human-readable message.
///
/// `code` is stable across releases and safe for agents to branch on;
/// `message` is informational and must never contain secrets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

/// The canonical application error type.
#[derive(Debug, Error)]
pub enum Error {
    /// A transfer or search backend is unreachable or not running.
    #[error("backend unavailable")]
    BackendUnavailable,

    /// Credentials were rejected or a required credential is missing.
    #[error("authentication failed")]
    AuthenticationFailed,

    /// The supplied transfer source (magnet/URL/torrent) is malformed.
    #[error("invalid source")]
    InvalidSource,

    /// A search has expired and its results are no longer available.
    #[error("search expired")]
    SearchExpired,

    /// A search result has expired and can no longer be downloaded.
    #[error("result expired")]
    ResultExpired,

    /// The requested transfer does not exist.
    #[error("transfer not found")]
    TransferNotFound,

    /// The requested search does not exist.
    #[error("search not found")]
    SearchNotFound,

    /// The requested result does not exist.
    #[error("result not found")]
    ResultNotFound,

    /// The requested resource does not exist.
    #[error("not found")]
    NotFound,

    /// The caller is not permitted to perform this action.
    #[error("permission denied")]
    PermissionDenied,

    /// A path escaped a configured root or otherwise failed validation.
    #[error("unsafe path")]
    UnsafePath,

    /// Archive extraction failed.
    #[error("extraction failed")]
    ExtractionFailed,

    /// Executable launch was denied by policy.
    #[error("process launch denied")]
    ProcessLaunchDenied,

    /// The managed sidecar version is unsupported.
    #[error("sidecar version unsupported")]
    SidecarVersionUnsupported,

    /// The operation conflicts with the current state.
    #[error("invalid state: {0}")]
    InvalidState(String),

    /// An opaque id could not be parsed (e.g. malformed UUID text).
    #[error("invalid id: {0}")]
    InvalidId(#[from] uuid::Error),

    /// A stored timestamp could not be parsed as RFC3339.
    #[error("invalid timestamp: {0}")]
    InvalidTimestamp(#[from] chrono::ParseError),

    /// A persistence failure occurred.
    #[error("database error: {0}")]
    Database(String),

    /// The backend rejected the operation.
    #[error("backend error: {0}")]
    Backend(String),

    /// A generic internal error.
    #[error("internal error: {0}")]
    Internal(String),
}

impl Error {
    /// Stable machine-readable code for this error.
    pub fn code(&self) -> &'static str {
        match self {
            Error::BackendUnavailable => "BackendUnavailable",
            Error::AuthenticationFailed => "AuthenticationFailed",
            Error::InvalidSource => "InvalidSource",
            Error::SearchExpired => "SearchExpired",
            Error::ResultExpired => "ResultExpired",
            Error::TransferNotFound => "TransferNotFound",
            Error::SearchNotFound => "SearchNotFound",
            Error::ResultNotFound => "ResultNotFound",
            Error::NotFound => "NotFound",
            Error::PermissionDenied => "PermissionDenied",
            Error::UnsafePath => "UnsafePath",
            Error::ExtractionFailed => "ExtractionFailed",
            Error::ProcessLaunchDenied => "ProcessLaunchDenied",
            Error::SidecarVersionUnsupported => "SidecarVersionUnsupported",
            Error::InvalidState(_) => "InvalidState",
            Error::InvalidId(_) => "InvalidId",
            Error::InvalidTimestamp(_) => "InvalidTimestamp",
            Error::Database(_) => "Database",
            Error::Backend(_) => "Backend",
            Error::Internal(_) => "Internal",
        }
    }

    /// Convert into a client-safe API error, stripping internal detail.
    pub fn into_api(self) -> ApiError {
        let code = self.code().to_string();
        let message = match &self {
            // Internal/database details are never exposed verbatim.
            Error::Database(_) | Error::Backend(_) | Error::Internal(_) => {
                "internal error".to_string()
            }
            other => other.to_string(),
        };
        ApiError { code, message }
    }
}

/// Convenience result alias used throughout the codebase.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable() {
        assert_eq!(Error::TransferNotFound.code(), "TransferNotFound");
        assert_eq!(Error::ProcessLaunchDenied.code(), "ProcessLaunchDenied");
        assert_eq!(Error::InvalidState("x".into()).code(), "InvalidState");
        assert_eq!(Error::Database("x".into()).code(), "Database");
    }

    #[test]
    fn invalid_id_converts_from_uuid_error() {
        let err: Error = "not-a-uuid".parse::<uuid::Uuid>().unwrap_err().into();
        assert_eq!(err.code(), "InvalidId");
        let api = err.into_api();
        assert_eq!(api.code, "InvalidId");
    }

    #[test]
    fn invalid_timestamp_converts_from_chrono_error() {
        let err: Error = chrono::DateTime::parse_from_rfc3339("garbage")
            .unwrap_err()
            .into();
        assert_eq!(err.code(), "InvalidTimestamp");
    }

    #[test]
    fn into_api_strips_internal_detail() {
        let api = Error::Database("secret details".into()).into_api();
        assert_eq!(api.code, "Database");
        assert_eq!(api.message, "internal error");
    }
}
