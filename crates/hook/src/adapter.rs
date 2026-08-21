//! Hook search backend adapter.
//!
//! Runs a user-configured command with the search query and parses magnet links
//! from its stdout. Supports two output contracts:
//!
//! - **Structured**: a JSON array, e.g.
//!   `[{"magnet": "magnet:?xt=urn:btih:...", "title": "My File", "size": 123}]`
//! - **Plain**: one magnet URI per line (optionally prefixed by a title,
//!   e.g. `My File | magnet:?xt=urn:btih:...`).

use agpeer_common::{
    Backend, Error, Result, ResultId, SearchBackend, SearchId, SearchRequest, SearchResult,
};
use agpeer_storage::{Database, SettingsStore};
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// In-memory search state owned by [`HookSearchBackend`].
#[derive(Default)]
struct BackendState {
    searches: HashMap<String, Vec<SearchResult>>,
}

/// A structured magnet hit emitted by a hook script.
#[derive(Debug, Deserialize)]
struct SearchHit {
    #[serde(default)]
    magnet: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    seeders: Option<u32>,
    #[serde(default)]
    leechers: Option<u32>,
}

/// Magnet-search backend backed by a user-configured external command.
pub struct HookSearchBackend {
    command: Vec<String>,
    timeout: Duration,
    max_results: usize,
    /// Optional handle to the settings DB, used to read the runtime
    /// `hook_search.enabled` and `hook_search.domains` settings at search time.
    db: Option<Database>,
    state: Arc<Mutex<BackendState>>,
}

impl HookSearchBackend {
    pub fn new(
        command: Vec<String>,
        timeout: Duration,
        max_results: usize,
        db: Option<Database>,
    ) -> Self {
        Self {
            command,
            timeout,
            max_results: max_results.max(1),
            db,
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
}

/// Split a command template into (program, args). `{query}` is substituted with
/// the search term and `{domains}` with the comma-joined domain list. When the
/// query or domains are not referenced by any argument, they are appended as
/// trailing positional arguments (query first, then domains). No shell is
/// involved, so all values are passed verbatim as single arguments.
fn build_command(template: &[String], query: &str, domains: &[String]) -> Option<(String, Vec<String>)> {
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
fn parse_output(bytes: &[u8], search_id: SearchId, max_results: usize) -> Vec<SearchResult> {
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
            if let Some(r) = hit.to_result(&search_id) {
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
            out.push(build_result(&search_id, &magnet, Some(title), None, None, None));
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

impl SearchHit {
    fn to_result(&self, search_id: &SearchId) -> Option<SearchResult> {
        let magnet = self.magnet.trim();
        if magnet.is_empty() {
            return None;
        }
        Some(build_result(
            search_id,
            magnet,
            self.title.clone(),
            self.size,
            self.seeders,
            self.leechers,
        ))
    }
}

/// Build a normalized search result for a magnet, preserving the magnet URI in
/// `backend_metadata["magnet"]` and `attributes["magnet"]`. Seeders/leechers
/// (when known) are stored under `attributes["seeders"]`/`["leechers"]`.
fn build_result(
    search_id: &SearchId,
    magnet: &str,
    title: Option<String>,
    size: Option<u64>,
    seeders: Option<u32>,
    leechers: Option<u32>,
) -> SearchResult {
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
        search_id: *search_id,
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

/// Minimal percent-decoding for URI-encoded display names.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
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
        let domains = self.domains().await;

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

        let parsed = parse_output(&stdout, id, self.max_results);
        if parsed.is_empty() {
            tracing::warn!(query = %request.query, "hook search parsed no magnets from output");
        }
        self.state
            .lock()
            .unwrap()
            .searches
            .insert(id.to_string(), parsed);
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
    use agpeer_common::SearchRequest;

    fn backend() -> HookSearchBackend {
        // No DB attached: `enabled()` returns true and `domains()` returns
        // empty, so these tests exercise parsing/command building in isolation.
        HookSearchBackend::new(Vec::new(), Duration::from_secs(5), 100, None)
    }

    #[test]
    fn parses_json_hits() {
        let id = SearchId::new();
        let out = parse_output(
            br#"[{"magnet":"magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","title":"Album/flac","size":1000,"seeders":123,"leechers":5}]"#,
            id,
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
        let id = SearchId::new();
        let out = parse_output(
            b"magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nmagnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nmagnet:?xt=urn:btih:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n",
            id,
            100,
        );
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn title_prefix_parsed() {
        let id = SearchId::new();
        let out = parse_output(
            b"Some Title | magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
            id,
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
        let id = SearchId::new();
        let out = parse_output(
            b"\n# comment\nmagnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
            id,
            100,
        );
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn respects_max_results() {
        let id = SearchId::new();
        let out = parse_output(
            b"magnet:?xt=urn:btih:a\nmagnet:?xt=urn:btih:b\nmagnet:?xt=urn:btih:c\n",
            id,
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
        let domains = ["nyaa.si".to_string(), "1337x.to".to_string()];
        // `{domains}` token substituted.
        let (_, args) = build_command(
            &["s".into(), "{query}".into(), "--domains".into(), "{domains}".into()],
            "q",
            &domains,
        )
        .unwrap();
        assert_eq!(args, vec!["q", "--domains", "nyaa.si,1337x.to"]);

        // No `{domains}` token: domains appended after the query.
        let (_, args2) = build_command(&["s".into(), "{query}".into()], "q", &domains).unwrap();
        assert_eq!(args2, vec!["q", "nyaa.si,1337x.to"]);

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
}
