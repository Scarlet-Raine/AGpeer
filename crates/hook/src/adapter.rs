//! Hook search backend adapter.
//!
//! Provides magnet search through two interchangeable paths, selected at search
//! time:
//!
//! - **Built-in** (no `command` configured): generic search-engine queries
//!   scoped to user-configured domains, plus optional user-configured site
//!   templates (see `crates/hook/src/builtin.rs`). Zero external files.
//! - **External command override** (`command` configured): the user's script is
//!   run per query with `{query}`/`{domains}` substitution. The script may emit
//!   a JSON array of hits or one magnet per line.

use crate::builtin::{generic_search, percent_decode, search_site, SearchHit};
use agpeer_common::{
    Backend, Error, HookSearchSite, Result, ResultId, SearchBackend, SearchId, SearchRequest,
    SearchResult,
};
use agpeer_storage::{Database, SettingsStore};
use async_trait::async_trait;
use reqwest::Client;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// In-memory search state owned by [`HookSearchBackend`].
#[derive(Default)]
struct BackendState {
    searches: HashMap<String, Vec<SearchResult>>,
}

/// Magnet-search backend: built-in domain-neutral search with an optional
/// external-command override.
pub struct HookSearchBackend {
    /// External command override. Empty = built-in search.
    command: Vec<String>,
    timeout: Duration,
    max_results: usize,
    /// Optional handle to the settings DB, used to read the runtime
    /// `hook_search.enabled`, `hook_search.domains`, and `hook_search.sites`
    /// settings at search time.
    db: Option<Database>,
    client: Client,
    state: Arc<Mutex<BackendState>>,
}

impl HookSearchBackend {
    pub fn new(
        command: Vec<String>,
        timeout: Duration,
        max_results: usize,
        db: Option<Database>,
    ) -> Self {
        let client = crate::builtin::http_client()
            .unwrap_or_else(|_| Client::builder().build().expect("reqwest client builds"));
        Self {
            command,
            timeout,
            max_results: max_results.max(1),
            db,
            client,
            state: Arc::new(Mutex::new(BackendState::default())),
        }
    }

    /// Whether the hook is runtime-enabled (defaults to enabled when no DB is
    /// attached, e.g. in unit tests).
    async fn enabled(&self) -> bool {
        let Some(db) = &self.db else {
            return true;
        };
        SettingsStore::new(db)
            .get_typed::<bool>("hook_search.enabled")
            .await
            .ok()
            .flatten()
            .unwrap_or(true)
    }

    /// The runtime-configured search domains (defaults to empty when no DB is
    /// attached, e.g. in unit tests).
    async fn domains(&self) -> Vec<String> {
        let Some(db) = &self.db else {
            return Vec::new();
        };
        SettingsStore::new(db)
            .get_typed::<Vec<String>>("hook_search.domains")
            .await
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    /// The runtime-configured site templates (defaults to empty when no DB is
    /// attached, e.g. in unit tests).
    async fn sites(&self) -> Vec<HookSearchSite> {
        let Some(db) = &self.db else {
            return Vec::new();
        };
        SettingsStore::new(db)
            .get_typed::<Vec<HookSearchSite>>("hook_search.sites")
            .await
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    /// Built-in search: generic engines + configured site templates, merged.
    /// The result budget for a search request: the caller's `max_results`,
    /// bounded by the backend's configured `max_results` (config safety cap).
    fn budget(&self, request: &SearchRequest) -> usize {
        request.max_results.max(1).min(self.max_results.max(1))
    }

    async fn builtin_search(&self, request: &SearchRequest) -> Result<Vec<SearchHit>> {
        let domains = self.domains().await;
        let sites = self.sites().await;
        let budget = self.budget(request);

        let mut hits = generic_search(&self.client, &request.query, &domains, budget).await?;
        let mut seen: HashSet<String> = hits.iter().map(|h| h.magnet.clone()).collect();

        for site in sites {
            // Stop fetching site templates once the result budget is already
            // satisfied: remaining work (incl. sequential detail-page fetches)
            // would be discarded and only add latency.
            if hits.len() >= budget {
                break;
            }
            if site.search.trim().is_empty() {
                continue;
            }
            match search_site(&self.client, &site, &request.query).await {
                Ok(mut page_hits) => {
                    for hit in page_hits.drain(..) {
                        if seen.insert(hit.magnet.clone()) {
                            hits.push(hit);
                            if hits.len() >= budget {
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        site = %site.domain,
                        error = %e,
                        "built-in search: site template failed"
                    );
                }
            }
        }

        hits.truncate(budget);
        Ok(hits)
    }

    /// External-command search (the "advanced override" path). A non-zero exit
    /// or timeout degrades to an empty result (a scraper may legitimately
    /// return "no hits"); a command that cannot be spawned is an error.
    async fn external_search(&self, request: &SearchRequest) -> Result<Vec<SearchResult>> {
        let domains = self.domains().await;
        let budget = self.budget(request);
        let stdout = match tokio::time::timeout(
            self.timeout,
            run_hook(&self.command, &request.query, &domains),
        )
        .await
        {
            Err(_) => {
                tracing::warn!("hook search timed out after {:?}", self.timeout);
                Vec::new()
            }
            Ok(Err(e)) => return Err(e),
            Ok(Ok(bytes)) => bytes,
        };
        let parsed = parse_output(&stdout, budget);
        if parsed.is_empty() {
            tracing::warn!(query = %request.query, "hook search parsed no magnets from output");
        }
        Ok(parsed)
    }
}

/// Split a command template into (program, args). `{query}` is substituted with
/// the search term and `{domains}` with the comma-joined domain list. When the
/// query or domains are not referenced by any argument, they are appended as
/// trailing positional arguments (query first, then domains). No shell is
/// involved, so all values are passed verbatim as single arguments.
fn build_command(
    template: &[String],
    query: &str,
    domains: &[String],
) -> Option<(String, Vec<String>)> {
    let domains_joined = domains.join(",");
    let mut program: Option<String> = None;
    let mut args: Vec<String> = Vec::new();
    let mut substituted_query = false;
    let mut substituted_domains = false;
    for part in template {
        if program.is_none() {
            program = Some(part.clone());
            continue;
        }
        let with_query = part.replace("{query}", query);
        if with_query != *part {
            substituted_query = true;
        }
        let with_domains = with_query.replace("{domains}", &domains_joined);
        if with_domains != with_query {
            substituted_domains = true;
        }
        args.push(with_domains);
    }
    let program = program?;
    if !substituted_query {
        args.push(query.to_string());
    }
    if !substituted_domains && !domains.is_empty() {
        args.push(domains_joined);
    }
    Some((program, args))
}

/// Run the hook command, returning its stdout bytes. Treats a non-zero exit or
/// time-out as an empty result rather than a hard failure (a scraper may
/// legitimately return "no hits"), but fails if the process cannot be spawned.
async fn run_hook(command: &[String], query: &str, domains: &[String]) -> Result<Vec<u8>> {
    let (program, args) = build_command(command, query, domains).ok_or_else(|| {
        Error::Backend("hook command is empty; configure [hook_search] command".to_string())
    })?;
    let output = match tokio::process::Command::new(&program)
        .args(&args)
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!(program = %program, error = %e, "hook search: command could not be spawned");
            return Err(Error::Backend(format!(
                "hook command could not be spawned: {e}"
            )));
        }
    };
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        tracing::warn!(
            program = %program,
            status = %output.status,
            stderr = %stderr,
            "hook search: command exited non-zero; treating as empty result"
        );
    } else if String::from_utf8_lossy(&output.stdout).trim().is_empty() {
        // A "successful" run that printed nothing is a silent failure from the
        // caller's perspective; surface any diagnostics the script wrote to
        // stderr so empty results are never mysterious.
        tracing::warn!(
            program = %program,
            stderr = %stderr,
            "hook search produced no output"
        );
    }
    Ok(output.stdout)
}

/// Parse hook stdout into normalized search results, deduplicated by magnet.
/// Structured JSON arrays and plain "magnet per line" output are both
/// supported.
fn parse_output(bytes: &[u8], max_results: usize) -> Vec<SearchResult> {
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim();
    let mut out: Vec<SearchResult> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    if let Some(hits) = parse_json_hits(trimmed) {
        for hit in hits {
            let magnet = hit.magnet.trim();
            if magnet.is_empty() || !seen.insert(magnet.to_string()) {
                continue;
            }
            if let Some(r) = hit_to_result(&hit, magnet) {
                out.push(r);
            }
        }
    } else {
        for line in trimmed.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((magnet, title)) = extract_magnet(line) else {
                continue;
            };
            if !seen.insert(magnet.clone()) {
                continue;
            }
            out.push(build_result(&magnet, Some(title), None, None, None));
        }
    }

    out.truncate(max_results);
    out
}

/// Try to parse stdout as a structured JSON array of hits.
fn parse_json_hits(trimmed: &str) -> Option<Vec<SearchHit>> {
    if !trimmed.starts_with('[') {
        return None;
    }
    serde_json::from_str::<Vec<SearchHit>>(trimmed).ok()
}

/// Normalize one parsed hit into a [`SearchResult`].
fn hit_to_result(hit: &SearchHit, magnet: &str) -> Option<SearchResult> {
    Some(build_result(
        magnet,
        hit.title.clone(),
        hit.size,
        hit.seeders,
        hit.leechers,
    ))
}

/// Build a normalized search result for a magnet, preserving the magnet URI in
/// `backend_metadata["magnet"]` and `attributes["magnet"]`. Seeders/leechers
/// (when known) are stored under `attributes["seeders"]`/`["leechers"]`.
fn build_result(
    magnet: &str,
    title: Option<String>,
    size: Option<u64>,
    seeders: Option<u32>,
    leechers: Option<u32>,
) -> SearchResult {
    let search_id = SearchId::new();
    let filename = title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| display_name(magnet));
    let extension = filename
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_lowercase())
        .filter(|ext| !ext.is_empty());

    let mut backend_metadata = HashMap::new();
    backend_metadata.insert("magnet".to_string(), serde_json::json!(magnet));
    let mut attributes = HashMap::new();
    attributes.insert("magnet".to_string(), serde_json::json!(magnet));
    if let Some(s) = seeders {
        attributes.insert("seeders".to_string(), serde_json::json!(s));
    }
    if let Some(l) = leechers {
        attributes.insert("leechers".to_string(), serde_json::json!(l));
    }

    SearchResult {
        result_id: ResultId::from(uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_URL,
            magnet.as_bytes(),
        )),
        search_id,
        username: String::new(),
        path: String::new(),
        filename,
        size,
        extension,
        bitrate: None,
        duration: None,
        attributes,
        queue_length: None,
        free_upload_slots: None,
        upload_speed: None,
        backend_metadata,
    }
}

/// Extract `(magnet_uri, optional_title)` from a plain output line. The title,
/// when present, precedes the magnet and is separated by whitespace and a
/// `|`, `,`, or tab.
fn extract_magnet(line: &str) -> Option<(String, String)> {
    let start = line.find("magnet:")?;
    let rest = &line[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let magnet = rest[..end].to_string();
    let title = line[..start]
        .trim()
        .trim_end_matches(['|', ',', '\t', ' '])
        .to_string();
    Some((magnet, title))
}

/// A human-readable fallback name derived from the magnet's `dn=` parameter or
/// its info hash.
fn display_name(magnet: &str) -> String {
    if let Some((_, name)) = magnet.split_once("dn=") {
        let name = name.split('&').next().unwrap_or("");
        if !name.is_empty() {
            return percent_decode(name);
        }
    }
    if let Some(pos) = magnet.find("btih:") {
        let hash = &magnet[pos + 5..];
        return format!("magnet ({})", &hash[..hash.len().min(8)]);
    }
    "magnet".to_string()
}

#[async_trait]
impl SearchBackend for HookSearchBackend {
    fn backend(&self) -> Backend {
        Backend::Hook
    }

    async fn search(&self, request: SearchRequest) -> Result<SearchId> {
        if request.backend != Backend::Hook {
            return Err(Error::InvalidSource);
        }
        if !self.enabled().await {
            return Err(Error::BackendUnavailable);
        }
        let id = SearchId::new();

        // Dispatch: an empty `command` uses the built-in path; a configured
        // `command` uses the external-script override.
        let mut results: Vec<SearchResult> = if self.command.is_empty() {
            let hits = self.builtin_search(&request).await?;
            hits.iter()
                .filter_map(|hit| {
                    let magnet = hit.magnet.trim();
                    if magnet.is_empty() {
                        return None;
                    }
                    Some(build_result(
                        magnet,
                        hit.title.clone(),
                        hit.size,
                        hit.seeders,
                        hit.leechers,
                    ))
                })
                .collect()
        } else {
            self.external_search(&request).await?
        };
        for result in &mut results {
            result.search_id = id;
        }
        if results.is_empty() {
            tracing::warn!(query = %request.query, "hook search returned no results");
        }
        self.state
            .lock()
            .unwrap()
            .searches
            .insert(id.to_string(), results);
        Ok(id)
    }

    async fn results(&self, id: &SearchId) -> Result<Vec<SearchResult>> {
        let state = self.state.lock().unwrap();
        state
            .searches
            .get(&id.to_string())
            .cloned()
            .ok_or(Error::SearchNotFound)
    }

    async fn stop(&self, id: &SearchId) -> Result<()> {
        self.state.lock().unwrap().searches.remove(&id.to_string());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend() -> HookSearchBackend {
        // No DB attached: `enabled()` returns true and `domains()` returns
        // empty, so these tests exercise parsing/command building in isolation.
        HookSearchBackend::new(Vec::new(), Duration::from_secs(5), 100, None)
    }

    #[test]
    fn parses_json_hits() {
        let out = parse_output(
            br#"[{"magnet":"magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","title":"Album/flac","size":1000,"seeders":123,"leechers":5}]"#,
            100,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].filename, "Album/flac");
        assert_eq!(out[0].size, Some(1000));
        assert_eq!(
            out[0]
                .backend_metadata
                .get("magnet")
                .unwrap()
                .as_str()
                .unwrap(),
            "magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(
            out[0].attributes.get("seeders").and_then(|v| v.as_u64()),
            Some(123)
        );
        assert_eq!(
            out[0].attributes.get("leechers").and_then(|v| v.as_u64()),
            Some(5)
        );
    }

    #[test]
    fn parses_plain_lines_and_dedups() {
        let out = parse_output(
            b"magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nmagnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nmagnet:?xt=urn:btih:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n",
            100,
        );
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn title_prefix_parsed() {
        let out = parse_output(
            b"Some Title | magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
            100,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].filename, "Some Title");
    }

    #[test]
    fn display_name_from_dn() {
        assert_eq!(
            display_name("magnet:?xt=urn:btih:aa&dn=My%20File"),
            "My File"
        );
        assert!(
            display_name("magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                .starts_with("magnet (")
        );
    }

    #[test]
    fn empty_and_comment_lines_ignored() {
        let out = parse_output(
            b"\n# comment\nmagnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
            100,
        );
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn respects_max_results() {
        let out = parse_output(
            b"magnet:?xt=urn:btih:a\nmagnet:?xt=urn:btih:b\nmagnet:?xt=urn:btih:c\n",
            2,
        );
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn build_command_substitutes_query_and_appends_when_missing() {
        let (prog, args) = build_command(
            &["scraper".into(), "--q".into(), "{query}".into()],
            "hello world",
            &[],
        )
        .unwrap();
        assert_eq!(prog, "scraper");
        assert_eq!(args, vec!["--q", "hello world"]);

        let (_, args2) = build_command(&["scraper".into()], "term", &[]).unwrap();
        assert_eq!(args2, vec!["term"]);
    }

    #[test]
    fn build_command_handles_domains() {
        let domains = [
            "search-index-1.example".to_string(),
            "search-index-2.example".to_string(),
        ];
        // `{domains}` token substituted.
        let (_, args) = build_command(
            &[
                "s".into(),
                "{query}".into(),
                "--domains".into(),
                "{domains}".into(),
            ],
            "q",
            &domains,
        )
        .unwrap();
        assert_eq!(
            args,
            vec![
                "q",
                "--domains",
                "search-index-1.example,search-index-2.example"
            ]
        );

        // No `{domains}` token: domains appended after the query.
        let (_, args2) = build_command(&["s".into(), "{query}".into()], "q", &domains).unwrap();
        assert_eq!(
            args2,
            vec!["q", "search-index-1.example,search-index-2.example"]
        );

        // No domains configured: nothing appended.
        let (_, args3) = build_command(&["s".into(), "{query}".into()], "q", &[]).unwrap();
        assert_eq!(args3, vec!["q"]);
    }

    #[tokio::test]
    async fn rejects_non_hook_backend() {
        let b = backend();
        let r = SearchRequest {
            backend: Backend::Soulseek,
            query: "x".into(),
            user: None,
            extension: None,
            min_size: None,
            max_results: 10,
        };
        let err = b.search(r).await.expect_err("non-hook backend must fail");
        assert!(matches!(err, agpeer_common::Error::InvalidSource));
    }

    /// An empty command must route to the built-in path (no "command is
    /// empty" error, no external process), degrading gracefully when offline.
    #[tokio::test]
    async fn builtin_search_dispatches_and_normalizes() {
        let b = backend();
        let r = SearchRequest {
            backend: Backend::Hook,
            query: "some query".into(),
            user: None,
            extension: None,
            min_size: None,
            max_results: 3,
        };
        let id = b.search(r).await.expect("built-in search should not fail");
        let results = b.results(&id).await.expect("results should exist");
        // Offline engine requests yield zero hits, never a hard error.
        assert_eq!(id.to_string().len(), 36);
        let _ = results;
    }
}
