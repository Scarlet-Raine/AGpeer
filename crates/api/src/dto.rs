use agpeer_common::ApiError;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BackendStatus {
    pub backend: String,
    pub transfer_available: bool,
    pub search_available: bool,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StatusResponse {
    pub version: String,
    pub uptime_secs: u64,
    pub server_time: DateTime<Utc>,
    pub db: String,
    pub backends: Vec<BackendStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MessageResponse {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
pub struct CancelRequest {
    pub delete_data: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SearchResponse {
    pub search_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DownloadDestination {
    pub destination: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteQuery {
    pub delete_data: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiErrorBody {
    pub code: String,
    pub message: String,
}

pub struct ApiErrorResponse(pub StatusCode, pub ApiError);

impl IntoResponse for ApiErrorResponse {
    fn into_response(self) -> Response {
        (self.0, Json(self.1)).into_response()
    }
}

pub fn err_to_response(e: agpeer_common::Error) -> ApiErrorResponse {
    let status = match &e {
        agpeer_common::Error::BackendUnavailable => StatusCode::SERVICE_UNAVAILABLE,
        agpeer_common::Error::AuthenticationFailed => StatusCode::UNAUTHORIZED,
        agpeer_common::Error::InvalidSource | agpeer_common::Error::UnsafePath => {
            StatusCode::BAD_REQUEST
        }
        agpeer_common::Error::SearchExpired | agpeer_common::Error::ResultExpired => {
            StatusCode::GONE
        }
        agpeer_common::Error::TransferNotFound
        | agpeer_common::Error::SearchNotFound
        | agpeer_common::Error::ResultNotFound
        | agpeer_common::Error::NotFound => StatusCode::NOT_FOUND,
        agpeer_common::Error::PermissionDenied | agpeer_common::Error::ProcessLaunchDenied => {
            StatusCode::FORBIDDEN
        }
        agpeer_common::Error::InvalidState(_) => StatusCode::CONFLICT,
        agpeer_common::Error::InvalidSetting(_) => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    ApiErrorResponse(status, e.into_api())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_not_found_maps_to_404() {
        assert_eq!(
            err_to_response(agpeer_common::Error::TransferNotFound).0,
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn backend_unavailable_maps_to_503() {
        assert_eq!(
            err_to_response(agpeer_common::Error::BackendUnavailable).0,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn process_launch_denied_maps_to_403() {
        assert_eq!(
            err_to_response(agpeer_common::Error::ProcessLaunchDenied).0,
            StatusCode::FORBIDDEN
        );
    }
}
