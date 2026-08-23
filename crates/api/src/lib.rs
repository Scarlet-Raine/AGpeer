//! agpeer HTTP API: Axum router, bearer auth, SSE event stream, and OpenAPI.

pub mod auth;
pub mod dto;
pub mod routes;
pub mod sse;
#[cfg(feature = "webui")]
pub mod webui;

pub use routes::router;
