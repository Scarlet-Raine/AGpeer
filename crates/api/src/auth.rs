use agpeer_common::ApiError;
use agpeer_core::state::AppState;
use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use axum::Json;
use std::sync::Arc;

pub struct BearerAuth;

#[derive(Debug)]
pub struct AuthRejection;

impl IntoResponse for AuthRejection {
    fn into_response(self) -> Response {
        (
            axum::http::StatusCode::UNAUTHORIZED,
            Json(ApiError::new(
                "AuthenticationFailed",
                "missing or invalid bearer token",
            )),
        )
            .into_response()
    }
}

pub(crate) fn token_matches(header: Option<&str>, expected: &str) -> bool {
    let Some(token) = header.and_then(|v| v.strip_prefix("Bearer ")) else {
        return false;
    };
    let t = token.trim();
    !t.is_empty() && t == expected
}

impl FromRequestParts<Arc<AppState>> for BearerAuth {
    type Rejection = AuthRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok());
        if token_matches(header, state.api_token.as_str()) {
            Ok(BearerAuth)
        } else {
            Err(AuthRejection)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agpeer_core::config::AppConfig;
    use agpeer_storage::Database;
    use axum::http::header::HeaderValue;

    async fn test_state(token: &str) -> Arc<AppState> {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let db = Database::from_pool(pool);
        AppState::new(AppConfig::default(), db, token.into())
    }

    #[tokio::test]
    async fn token_matches_accepts_exact_token() {
        let state = test_state("fixed-token").await;
        assert!(token_matches(
            Some("Bearer fixed-token"),
            state.api_token.as_str()
        ));
        assert!(!token_matches(
            Some("Bearer other"),
            state.api_token.as_str()
        ));
    }

    #[tokio::test]
    async fn token_matches_rejects_missing_and_malformed() {
        assert!(!token_matches(None, "secret"));
        assert!(!token_matches(Some("Basic abc"), "secret"));
        assert!(!token_matches(Some("Bearer"), "secret"));
        assert!(!token_matches(Some("Bearer "), "secret"));
        assert!(!token_matches(Some(""), "secret"));
    }

    #[tokio::test]
    async fn token_matches_trims_surrounding_whitespace() {
        assert!(token_matches(Some("Bearer secret-token "), "secret-token"));
        assert!(token_matches(Some("Bearer  secret-token"), "secret-token"));
    }

    #[tokio::test]
    async fn extractor_accepts_valid_token() {
        let state = test_state("fixed-token").await;
        let request = axum::http::Request::builder()
            .header(
                AUTHORIZATION,
                HeaderValue::from_static("Bearer fixed-token"),
            )
            .body(())
            .unwrap();
        let (mut parts, _) = request.into_parts();
        assert!(BearerAuth::from_request_parts(&mut parts, &state)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn extractor_rejects_invalid_token() {
        let state = test_state("fixed-token").await;
        let request = axum::http::Request::builder()
            .header(AUTHORIZATION, HeaderValue::from_static("Bearer wrong"))
            .body(())
            .unwrap();
        let (mut parts, _) = request.into_parts();
        assert!(BearerAuth::from_request_parts(&mut parts, &state)
            .await
            .is_err());
    }
}
