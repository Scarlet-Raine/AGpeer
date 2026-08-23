//! One-binary WebUI: the desktop UI's `dist` is embedded into the core binary
//! (feature `webui`, off by default) and served from `GET /`. Only active when
//! the feature is enabled.
//!
//! - `GET /` and any non-`api/` path serve the SPA (client-side routing);
//! - `api/`-prefixed misses return JSON 404s so API tooling is never confused
//!   by an HTML fallback;
//! - `GET /__agpeer_token` bootstraps the browser UI with the bearer token and
//!   is **loopback-only** (any other peer gets 403);
//! - setting `AGPEER_UI_TOKEN_INJECT=1` injects `window.__AGPEER_TOKEN__` into
//!   the served page instead — required for container/LAN setups where the
//!   loopback-only endpoint can't reach a remote browser. This widens exposure
//!   of the token to anyone who can reach the server, so it is opt-in and
//!   documented as such.

use agpeer_core::state::AppState;
use axum::body::Body;
use axum::extract::{ConnectInfo, State};
use axum::http::header::{self, HeaderValue};
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use std::borrow::Cow;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::OnceLock;

#[derive(rust_embed::RustEmbed)]
#[folder = "../../apps/desktop/dist"]
struct Assets;

/// Whether the served page should embed the bearer token
/// (`window.__AGPEER_TOKEN__`), for container/LAN UI bootstrap. Read once per
/// process.
fn inject_token() -> bool {
    static INJECT: OnceLock<bool> = OnceLock::new();
    *INJECT.get_or_init(|| {
        std::env::var("AGPEER_UI_TOKEN_INJECT")
            .map(|v| v == "1")
            .unwrap_or(false)
    })
}

/// `GET /` — the embedded UI.
pub async fn index(State(state): State<Arc<AppState>>) -> Response {
    serve_asset("index.html", &state)
}

/// `GET /__agpeer_token` — loopback-only token bootstrap for the browser UI.
/// Never cross-origin readable: the webui routes are registered outside the
/// API CORS layer, so only same-origin pages (or CORS-enabled callers) reach
/// it.
pub async fn token(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Response {
    if !peer.ip().is_loopback() {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "code": "PermissionDenied",
                "message": "token bootstrap is loopback-only"
            })),
        )
            .into_response();
    }
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; charset=utf-8"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (header::PRAGMA, HeaderValue::from_static("no-cache")),
        ],
        state.api_token.as_str().to_string(),
    )
        .into_response()
}

/// SPA fallback: real embedded assets are served directly; `api`-prefixed
/// paths (whether exact `/api` or `/api/v1/...`) get a JSON 404; anything
/// else gets the app shell.
pub async fn spa_fallback(State(state): State<Arc<AppState>>, uri: Uri) -> Response {
    let cleaned = uri.path().trim_start_matches('/');
    if !cleaned.is_empty() && Assets::get(cleaned).is_some() {
        return serve_asset(cleaned, &state);
    }
    if is_api_miss(uri.path()) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"code": "NotFound", "message": "api endpoint not found"})),
        )
            .into_response();
    }
    serve_asset("index.html", &state)
}

/// Whether a path is an API miss (its first path segment is `api`). Matches
/// exact `/api` and `/api/v1/...` alike, never merely paths whose string
/// starts with `api/`.
fn is_api_miss(path: &str) -> bool {
    path.split('/').nth(1) == Some("api")
}

/// Serve an embedded asset. Non-HTML assets are served zero-copy from the
/// embedded `'static` bytes; `index.html` is rebuilt only when token
/// injection is enabled (with the token JSON-escaping-safe).
fn serve_asset(path: &str, state: &Arc<AppState>) -> Response {
    let Some(file) = Assets::get(path) else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    let mime = mime_for(path);
    let cache = if path == "index.html" {
        "no-store"
    } else {
        // Vite content-hashes asset filenames, so long-lived caching is safe.
        "public, max-age=31536000, immutable"
    };
    let headers = [
        (header::CONTENT_TYPE, HeaderValue::from_static(mime)),
        (header::CACHE_CONTROL, HeaderValue::from_static(cache)),
    ];

    if path == "index.html" {
        let body = index_html(&file.data, &state.api_token);
        return (StatusCode::OK, headers, Body::from(body)).into_response();
    }
    let body = match &file.data {
        Cow::Borrowed(bytes) => Body::from(axum::body::Bytes::from_static(bytes)),
        Cow::Owned(vec) => Body::from(axum::body::Bytes::from(vec.clone())),
    };
    (StatusCode::OK, headers, body).into_response()
}

/// Build the served `index.html` body, optionally embedding the token.
///
/// The token is serialized as a JSON string literal so a token containing
/// quotes/backslashes/`</script>` can never escape the script context.
fn index_html(data: &[u8], token: &str) -> Vec<u8> {
    if !inject_token() {
        return data.to_vec();
    }
    let Ok(mut html) = String::from_utf8(data.to_vec()) else {
        return data.to_vec();
    };
    if let Some(position) = html.rfind("</body>") {
        html.insert_str(position, &token_script(token));
    }
    html.into_bytes()
}

/// The `<script>` snippet embedding the token as a JSON-escaped string
/// literal.
fn token_script(token: &str) -> String {
    format!(
        "<script>window.__AGPEER_TOKEN__={};</script>",
        serde_json::to_string(token).expect("a string always serializes")
    )
}

fn mime_for(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "txt" => "text/plain; charset=utf-8",
        "webmanifest" => "application/manifest+json",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_script_is_json_escaped() {
        // A hostile token (quotes, backslash, closing script tag) must not be
        // able to leave the string literal.
        let script = token_script("t\"</script>ok\\n");
        assert!(script.contains(r#""t\"</script>ok\\n""#));
        // No raw `</script>` inside the string portion of the script body.
        let value = script.trim_end_matches(";</script>");
        assert!(!value.contains("</script>"));
    }

    #[test]
    fn api_miss_path_matches_exact_api_prefix() {
        assert!(is_api_miss("/api"));
        assert!(is_api_miss("/api/v1"));
        assert!(is_api_miss("/api/v1/nope"));
        assert!(!is_api_miss("/"));
        assert!(!is_api_miss("/assets/app.js"));
    }

    #[test]
    fn mime_mapping_is_stable() {
        assert_eq!(mime_for("index.html"), "text/html; charset=utf-8");
        assert_eq!(
            mime_for("assets/index-abc123.js"),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(mime_for("assets/app.css"), "text/css; charset=utf-8");
        assert_eq!(mime_for("favicon.ico"), "image/x-icon");
        assert_eq!(mime_for("data.bin"), "application/octet-stream");
    }
}
