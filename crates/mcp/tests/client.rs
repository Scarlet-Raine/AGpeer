//! Integration tests for the agpeer MCP REST client against a mock agpeer API.
//!
//! These verify the client really talks the documented `/api/v1` surface:
//! bearer auth, JSON request/response, and typed error mapping. No real agpeer
//! core or network is required.

use agpeer_mcp::AgpeerClient;
use httpmock::prelude::*;
use httpmock::Method::{DELETE, GET, POST};
use serde_json::json;

#[tokio::test]
async fn status_sends_bearer_token_and_parses() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET)
            .path("/api/v1/status")
            .header("authorization", "Bearer secret-token");
        then.status(200).json_body(json!({
            "version": "0.1.0",
            "uptime_secs": 42,
            "db": "ok",
            "backends": []
        }));
    });

    let client = AgpeerClient::new(server.base_url(), "secret-token");
    let status = client.status().await.expect("status ok");
    assert_eq!(status["version"], "0.1.0");
    assert_eq!(status["uptime_secs"], 42);
}

#[tokio::test]
async fn add_transfer_posts_json() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(POST)
            .path("/api/v1/transfers")
            .json_body(json!({
                "backend": "torrent",
                "source": "magnet:?xt=urn:btih:abc"
            }));
        then.status(201).json_body(json!({ "transfer_id": "t-1" }));
    });

    let client = AgpeerClient::new(server.base_url(), "tok");
    let resp = client
        .add_transfer(json!({ "backend": "torrent", "source": "magnet:?xt=urn:btih:abc" }))
        .await
        .expect("add ok");
    assert_eq!(resp["transfer_id"], "t-1");
}

#[tokio::test]
async fn missing_transfer_maps_to_typed_http_error() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/api/v1/transfers/nope");
        then.status(404).json_body(json!({
            "code": "TransferNotFound",
            "message": "transfer not found"
        }));
    });

    let client = AgpeerClient::new(server.base_url(), "tok");
    let err = client.get_transfer("nope").await.expect_err("should fail");
    match err {
        agpeer_mcp::ClientError::Http { status, body } => {
            assert_eq!(status, 404);
            assert!(body.contains("TransferNotFound"));
        }
        other => panic!("expected Http error, got {other:?}"),
    }
}

#[tokio::test]
async fn delete_transfer_uses_body() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(DELETE)
            .path("/api/v1/transfers/t-1")
            .body(r#"{"delete_data":true}"#);
        then.status(200)
            .json_body(json!({ "message": "transfer removed" }));
    });

    let client = AgpeerClient::new(server.base_url(), "tok");
    let resp = client.delete_transfer("t-1", true).await.expect("ok");
    assert_eq!(resp["message"], "transfer removed");
}
