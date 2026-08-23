//! `agpeer-mcp` — a thin MCP server over the agpeer `/api/v1` REST API.
//!
//! The MCP server carries no business logic of its own: every tool maps
//! one-to-one onto a documented agpeer REST endpoint. See `docs/api.md` and
//! the AGENTS.md "MCP" section for the intended contract.

use std::collections::HashMap;
use std::time::Duration;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::schemars::JsonSchema;
use rmcp::{tool, tool_router, ErrorData};
use serde::Deserialize;
use serde_json::{json, Value};

/// Error returned by the agpeer REST client.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("agpeer responded with {status}: {body}")]
    Http { status: u16, body: String },
    #[error("unexpected response: {0}")]
    Decode(String),
}

/// Minimal HTTP client for the agpeer REST API.
///
/// The core service is the product; this client only speaks its public,
/// versioned HTTP surface. All requests carry the bearer token.
#[derive(Clone)]
pub struct AgpeerClient {
    base: String,
    token: String,
    http: reqwest::Client,
}

impl AgpeerClient {
    /// `api_base` may or may not include a trailing slash, e.g.
    /// `http://127.0.0.1:41000` or `http://127.0.0.1:41000/`.
    pub fn new(api_base: impl Into<String>, token: impl Into<String>) -> Self {
        let base = api_base.into().trim_end_matches('/').to_string();
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .build()
            .expect("build reqwest client");
        Self {
            base,
            token: token.into(),
            http,
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api/v1{}", self.base, path)
    }

    async fn get(&self, path: &str) -> Result<Value, ClientError> {
        let req = self
            .http
            .get(self.url(path))
            .bearer_auth(&self.token)
            .send()
            .await?;
        self.decode(req).await
    }

    async fn post(&self, path: &str, body: Option<Value>) -> Result<Value, ClientError> {
        let mut builder = self.http.post(self.url(path)).bearer_auth(&self.token);
        if let Some(b) = body {
            builder = builder.json(&b);
        }
        self.decode(builder.send().await?).await
    }

    async fn delete_body(&self, path: &str, body: Value) -> Result<Value, ClientError> {
        let req = self
            .http
            .delete(self.url(path))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await?;
        self.decode(req).await
    }

    async fn delete(&self, path: &str) -> Result<Value, ClientError> {
        let req = self
            .http
            .delete(self.url(path))
            .bearer_auth(&self.token)
            .send()
            .await?;
        self.decode(req).await
    }

    async fn put(&self, path: &str, body: Value) -> Result<Value, ClientError> {
        let req = self
            .http
            .put(self.url(path))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await?;
        self.decode(req).await
    }

    async fn decode(&self, resp: reqwest::Response) -> Result<Value, ClientError> {
        let status = resp.status();
        let text = resp.text().await?;
        if status.is_success() {
            if text.trim().is_empty() {
                return Ok(Value::Null);
            }
            serde_json::from_str(&text).map_err(|e| ClientError::Decode(e.to_string()))
        } else {
            Err(ClientError::Http {
                status: status.as_u16(),
                body: text,
            })
        }
    }

    pub async fn status(&self) -> Result<Value, ClientError> {
        self.get("/status").await
    }

    pub async fn backends(&self) -> Result<Value, ClientError> {
        self.get("/backends").await
    }

    pub async fn list_transfers(&self) -> Result<Value, ClientError> {
        self.get("/transfers").await
    }

    pub async fn get_transfer(&self, id: &str) -> Result<Value, ClientError> {
        self.get(&format!("/transfers/{id}")).await
    }

    pub async fn transfer_files(&self, id: &str) -> Result<Value, ClientError> {
        self.get(&format!("/transfers/{id}/files")).await
    }

    pub async fn add_transfer(&self, request: Value) -> Result<Value, ClientError> {
        self.post("/transfers", Some(request)).await
    }

    pub async fn pause_transfer(&self, id: &str) -> Result<Value, ClientError> {
        self.post(&format!("/transfers/{id}/pause"), None).await
    }

    pub async fn resume_transfer(&self, id: &str) -> Result<Value, ClientError> {
        self.post(&format!("/transfers/{id}/resume"), None).await
    }

    pub async fn cancel_transfer(&self, id: &str, delete_data: bool) -> Result<Value, ClientError> {
        self.post(
            &format!("/transfers/{id}/cancel"),
            Some(json!({ "delete_data": delete_data })),
        )
        .await
    }

    pub async fn delete_transfer(&self, id: &str, delete_data: bool) -> Result<Value, ClientError> {
        self.delete_body(
            &format!("/transfers/{id}"),
            json!({ "delete_data": delete_data }),
        )
        .await
    }

    pub async fn list_searches(&self) -> Result<Value, ClientError> {
        self.get("/searches").await
    }

    pub async fn start_search(&self, request: Value) -> Result<Value, ClientError> {
        self.post("/searches", Some(request)).await
    }

    pub async fn get_search(&self, id: &str) -> Result<Value, ClientError> {
        self.get(&format!("/searches/{id}")).await
    }

    pub async fn search_results(&self, id: &str) -> Result<Value, ClientError> {
        self.get(&format!("/searches/{id}/results")).await
    }

    pub async fn stop_search(&self, id: &str) -> Result<Value, ClientError> {
        self.post(&format!("/searches/{id}/stop"), None).await
    }

    pub async fn download_result(
        &self,
        search_id: &str,
        result_id: &str,
        destination: Option<String>,
    ) -> Result<Value, ClientError> {
        let body = match destination {
            Some(d) => json!({ "destination": d }),
            None => Value::Null,
        };
        self.post(
            &format!("/searches/{search_id}/results/{result_id}/download"),
            if body.is_null() { None } else { Some(body) },
        )
        .await
    }

    pub async fn list_postprocess(&self) -> Result<Value, ClientError> {
        self.get("/postprocess").await
    }

    pub async fn get_postprocess(&self, id: &str) -> Result<Value, ClientError> {
        self.get(&format!("/postprocess/{id}")).await
    }

    pub async fn list_library(&self) -> Result<Value, ClientError> {
        self.get("/library").await
    }

    pub async fn get_settings(&self) -> Result<Value, ClientError> {
        self.get("/settings").await
    }

    pub async fn put_settings(&self, settings: Value) -> Result<Value, ClientError> {
        self.put("/settings", settings).await
    }

    pub async fn get_setting(&self, key: &str) -> Result<Value, ClientError> {
        self.get(&format!("/settings/{key}")).await
    }

    pub async fn put_setting(&self, key: &str, value: Value) -> Result<Value, ClientError> {
        self.put(&format!("/settings/{key}"), value).await
    }

    pub async fn delete_setting(&self, key: &str) -> Result<Value, ClientError> {
        self.delete(&format!("/settings/{key}")).await
    }
}

fn err_to_data(e: ClientError) -> ErrorData {
    ErrorData::internal_error(e.to_string(), None)
}

fn pretty(v: Value) -> Result<String, ErrorData> {
    serde_json::to_string_pretty(&v).map_err(|e| err_to_data(ClientError::Decode(e.to_string())))
}

async fn call<F, T>(f: F) -> Result<String, ErrorData>
where
    F: FnOnce() -> T,
    T: std::future::Future<Output = Result<Value, ClientError>>,
{
    pretty(f().await.map_err(err_to_data)?)
}

// ---------------------------------------------------------------------------
// Tool input payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, JsonSchema, Default)]
pub struct AddTransferInput {
    /// `torrent` or `soulseek`.
    pub backend: String,
    /// Magnet URI, local `.torrent` path, remote `.torrent` URL, or a
    /// `soulseek:` result id.
    pub source: String,
    /// Optional destination directory. Defaults to the configured root.
    #[serde(default)]
    pub destination: Option<String>,
    /// Optional display name override.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Optional per-file selection (torrent backends): `[{"index": "0", "selected": true}]`.
    #[serde(default)]
    pub file_selection: Option<Vec<FileSelectionInput>>,
    /// Optional metadata to attach to the transfer.
    #[serde(default)]
    pub metadata: Option<HashMap<String, Value>>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, Default)]
pub struct FileSelectionInput {
    pub index: String,
    pub selected: bool,
}

impl From<AddTransferInput> for Value {
    fn from(p: AddTransferInput) -> Self {
        let mut map = serde_json::Map::new();
        map.insert("backend".into(), Value::String(p.backend));
        map.insert("source".into(), Value::String(p.source));
        if let Some(d) = p.destination {
            map.insert("destination".into(), Value::String(d));
        }
        if let Some(n) = p.display_name {
            map.insert("display_name".into(), Value::String(n));
        }
        if let Some(fs) = p.file_selection {
            let v: Vec<Value> = fs
                .into_iter()
                .map(|f| json!({"index": f.index, "selected": f.selected}))
                .collect();
            map.insert("file_selection".into(), Value::Array(v));
        }
        if let Some(m) = p.metadata {
            map.insert("metadata".into(), Value::Object(m.into_iter().collect()));
        }
        Value::Object(map)
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema, Default)]
pub struct IdInput {
    pub id: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, Default)]
pub struct DeleteTransferInput {
    pub id: String,
    /// Delete downloaded files too (default false).
    #[serde(default)]
    pub delete_data: bool,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, Default)]
pub struct SearchInput {
    /// `soulseek` or `hook` (a user-configured magnet-search command).
    pub backend: String,
    pub query: String,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub extension: Option<String>,
    #[serde(default)]
    pub min_size: Option<u64>,
    #[serde(default)]
    pub max_results: Option<usize>,
}

impl From<SearchInput> for Value {
    fn from(p: SearchInput) -> Self {
        let mut map = serde_json::Map::new();
        map.insert("backend".into(), Value::String(p.backend));
        map.insert("query".into(), Value::String(p.query));
        if let Some(u) = p.user {
            map.insert("user".into(), Value::String(u));
        }
        if let Some(e) = p.extension {
            map.insert("extension".into(), Value::String(e));
        }
        if let Some(s) = p.min_size {
            map.insert("min_size".into(), json!(s));
        }
        if let Some(m) = p.max_results {
            map.insert("max_results".into(), json!(m));
        }
        Value::Object(map)
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema, Default)]
pub struct DownloadResultInput {
    pub search_id: String,
    pub result_id: String,
    #[serde(default)]
    pub destination: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, Default)]
pub struct SettingsMapInput {
    /// Settings to set, e.g. `{"hook_search.enabled": true}`.
    pub settings: HashMap<String, Value>,
}

impl From<SettingsMapInput> for Value {
    fn from(p: SettingsMapInput) -> Self {
        Value::Object(p.settings.into_iter().collect())
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema, Default)]
pub struct KeyInput {
    pub key: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SettingValueInput {
    pub key: String,
    /// JSON value to store for this setting.
    pub value: Value,
}

// ---------------------------------------------------------------------------
// MCP server
// ---------------------------------------------------------------------------

/// The MCP server. Holds an [`AgpeerClient`] and exposes one tool per REST
/// endpoint. Stateless with respect to the protocol session.
#[derive(Clone)]
pub struct AgpeerServer {
    client: AgpeerClient,
}

impl AgpeerServer {
    pub fn new(client: AgpeerClient) -> Self {
        Self { client }
    }

    pub fn client(&self) -> &AgpeerClient {
        &self.client
    }
}

#[tool_router(server_handler)]
impl AgpeerServer {
    #[tool(description = "Fetch agpeer core health: version, uptime, database state.")]
    async fn status(&self) -> Result<String, ErrorData> {
        call(|| self.client.status()).await
    }

    #[tool(description = "List registered backends and their runtime state.")]
    async fn list_backends(&self) -> Result<String, ErrorData> {
        call(|| self.client.backends()).await
    }

    #[tool(description = "List all transfers (normalized across torrent and soulseek).")]
    async fn list_transfers(&self) -> Result<String, ErrorData> {
        call(|| self.client.list_transfers()).await
    }

    #[tool(description = "Fetch a single transfer by id.")]
    async fn get_transfer(&self, Parameters(p): Parameters<IdInput>) -> Result<String, ErrorData> {
        call(|| self.client.get_transfer(&p.id)).await
    }

    #[tool(
        description = "Add a transfer. Use backend \"torrent\" with a magnet / .torrent / URL source, or a magnet returned by a hook search; use \"soulseek\" with a soulseek: result id."
    )]
    async fn add_transfer(
        &self,
        Parameters(p): Parameters<AddTransferInput>,
    ) -> Result<String, ErrorData> {
        call(|| self.client.add_transfer(p.into())).await
    }

    #[tool(description = "Pause a transfer by id.")]
    async fn pause_transfer(
        &self,
        Parameters(p): Parameters<IdInput>,
    ) -> Result<String, ErrorData> {
        call(|| self.client.pause_transfer(&p.id)).await
    }

    #[tool(description = "Resume a transfer by id.")]
    async fn resume_transfer(
        &self,
        Parameters(p): Parameters<IdInput>,
    ) -> Result<String, ErrorData> {
        call(|| self.client.resume_transfer(&p.id)).await
    }

    #[tool(description = "Cancel a transfer by id. Optionally delete downloaded files.")]
    async fn cancel_transfer(
        &self,
        Parameters(p): Parameters<DeleteTransferInput>,
    ) -> Result<String, ErrorData> {
        call(|| self.client.cancel_transfer(&p.id, p.delete_data)).await
    }

    #[tool(description = "Remove a transfer job. Optionally delete downloaded files.")]
    async fn delete_transfer(
        &self,
        Parameters(p): Parameters<DeleteTransferInput>,
    ) -> Result<String, ErrorData> {
        call(|| self.client.delete_transfer(&p.id, p.delete_data)).await
    }

    #[tool(description = "List a transfer's files with selection status.")]
    async fn list_transfer_files(
        &self,
        Parameters(p): Parameters<IdInput>,
    ) -> Result<String, ErrorData> {
        call(|| self.client.transfer_files(&p.id)).await
    }

    #[tool(description = "List searches.")]
    async fn list_searches(&self) -> Result<String, ErrorData> {
        call(|| self.client.list_searches()).await
    }

    #[tool(
        description = "Start a search on a search-enabled backend (soulseek or hook magnet search)."
    )]
    async fn start_search(
        &self,
        Parameters(p): Parameters<SearchInput>,
    ) -> Result<String, ErrorData> {
        call(|| self.client.start_search(p.into())).await
    }

    #[tool(description = "Fetch a search's status.")]
    async fn get_search(&self, Parameters(p): Parameters<IdInput>) -> Result<String, ErrorData> {
        call(|| self.client.get_search(&p.id)).await
    }

    #[tool(description = "Fetch accumulated results for a search.")]
    async fn get_search_results(
        &self,
        Parameters(p): Parameters<IdInput>,
    ) -> Result<String, ErrorData> {
        call(|| self.client.search_results(&p.id)).await
    }

    #[tool(description = "Stop an in-progress search.")]
    async fn stop_search(&self, Parameters(p): Parameters<IdInput>) -> Result<String, ErrorData> {
        call(|| self.client.stop_search(&p.id)).await
    }

    #[tool(description = "Download a Soulseek search result as a transfer.")]
    async fn download_search_result(
        &self,
        Parameters(p): Parameters<DownloadResultInput>,
    ) -> Result<String, ErrorData> {
        call(|| {
            self.client
                .download_result(&p.search_id, &p.result_id, p.destination)
        })
        .await
    }

    #[tool(description = "List post-processing jobs.")]
    async fn list_postprocess_jobs(&self) -> Result<String, ErrorData> {
        call(|| self.client.list_postprocess()).await
    }

    #[tool(description = "Fetch a post-processing job with its step states.")]
    async fn get_postprocess_job(
        &self,
        Parameters(p): Parameters<IdInput>,
    ) -> Result<String, ErrorData> {
        call(|| self.client.get_postprocess(&p.id)).await
    }

    #[tool(
        description = "List the organized media library (files under the configured library root). Empty when no library root is configured."
    )]
    async fn list_library(&self) -> Result<String, ErrorData> {
        call(|| self.client.list_library()).await
    }

    #[tool(description = "List runtime settings (secrets are never returned).")]
    async fn get_settings(&self) -> Result<String, ErrorData> {
        call(|| self.client.get_settings()).await
    }

    #[tool(
        description = "Set multiple runtime settings at once. Returns the full settings map. Secrets are never settable through this API."
    )]
    async fn put_settings(
        &self,
        Parameters(p): Parameters<SettingsMapInput>,
    ) -> Result<String, ErrorData> {
        call(|| self.client.put_settings(p.into())).await
    }

    #[tool(description = "Fetch a single runtime setting by key.")]
    async fn get_setting(&self, Parameters(p): Parameters<KeyInput>) -> Result<String, ErrorData> {
        call(|| self.client.get_setting(&p.key)).await
    }

    #[tool(description = "Set one runtime setting by key. The value is stored as given JSON.")]
    async fn put_setting(
        &self,
        Parameters(p): Parameters<SettingValueInput>,
    ) -> Result<String, ErrorData> {
        call(|| self.client.put_setting(&p.key, p.value)).await
    }

    #[tool(description = "Delete a runtime setting by key, restoring its default.")]
    async fn delete_setting(
        &self,
        Parameters(p): Parameters<KeyInput>,
    ) -> Result<String, ErrorData> {
        call(|| self.client.delete_setting(&p.key)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_transfer_payload_builds() {
        let input = AddTransferInput {
            backend: "torrent".into(),
            source: "magnet:?xt=urn:btih:abc".into(),
            destination: Some("D:\\Downloads".into()),
            display_name: Some("thing".into()),
            file_selection: Some(vec![FileSelectionInput {
                index: "0".into(),
                selected: true,
            }]),
            metadata: Some(HashMap::from([("k".to_string(), json!("v"))])),
        };
        let v: Value = input.into();
        assert_eq!(v["backend"], "torrent");
        assert_eq!(v["source"], "magnet:?xt=urn:btih:abc");
        assert_eq!(v["destination"], "D:\\Downloads");
        assert_eq!(v["file_selection"][0]["index"], "0");
        assert_eq!(v["metadata"]["k"], "v");
    }

    #[test]
    fn search_payload_omits_defaults() {
        let input = SearchInput {
            backend: "soulseek".into(),
            query: "flac".into(),
            ..Default::default()
        };
        let v: Value = input.into();
        assert_eq!(v["backend"], "soulseek");
        assert!(v.get("user").is_none());
        assert!(v.get("min_size").is_none());
    }

    #[test]
    fn base_url_trailing_slash_normalized() {
        let c = AgpeerClient::new("http://127.0.0.1:41000/", "t");
        assert_eq!(c.base_url(), "http://127.0.0.1:41000");
        assert_eq!(c.url("/status"), "http://127.0.0.1:41000/api/v1/status");
    }

    #[test]
    fn tool_router_exposes_expected_tools() {
        let router = AgpeerServer::tool_router();
        let names: Vec<String> = router
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        for expected in [
            "status",
            "list_backends",
            "list_transfers",
            "get_transfer",
            "add_transfer",
            "pause_transfer",
            "resume_transfer",
            "cancel_transfer",
            "delete_transfer",
            "list_transfer_files",
            "list_searches",
            "start_search",
            "get_search",
            "get_search_results",
            "stop_search",
            "download_search_result",
            "list_postprocess_jobs",
            "get_postprocess_job",
            "list_library",
            "get_settings",
            "put_settings",
            "get_setting",
            "put_setting",
            "delete_setting",
        ] {
            assert!(
                names.iter().any(|n| n == expected),
                "tool registry missing {expected}; got {names:?}"
            );
        }
    }
}
