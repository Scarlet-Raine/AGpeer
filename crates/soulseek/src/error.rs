//! Error mapping for the Soulseek backend adapter.
//!
//! Converts [`rustsoseek::Error`] into [`agpeer_common::Error`] so the rest of
//! the application only ever sees the shared error vocabulary.

/// Convert a [`rustsoseek::Error`] into the shared [`agpeer_common::Error`].
pub fn map_error(e: rustsoseek::Error) -> agpeer_common::Error {
    match e {
        rustsoseek::Error::Unavailable(_) => agpeer_common::Error::BackendUnavailable,
        rustsoseek::Error::AuthenticationFailed => agpeer_common::Error::AuthenticationFailed,
        rustsoseek::Error::Invalid(m) => agpeer_common::Error::Backend(m),
        rustsoseek::Error::Io(m) => agpeer_common::Error::Backend(m),
        rustsoseek::Error::Internal(m) => agpeer_common::Error::Backend(m),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_maps_to_backend_unavailable() {
        assert_eq!(
            map_error(rustsoseek::Error::Unavailable("down".into())).code(),
            "BackendUnavailable"
        );
    }

    #[test]
    fn authentication_maps_to_authentication_failed() {
        assert_eq!(
            map_error(rustsoseek::Error::AuthenticationFailed).code(),
            "AuthenticationFailed"
        );
    }
}
