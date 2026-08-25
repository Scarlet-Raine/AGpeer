//! Built-in magnet search (domain-neutral).
//!
//! This is the default magnet-search path when no external `command` is
//! configured. It queries generic search engines with a `site:<domain>`- and
//! `magnet:`-scoped query per user-configured domain, and additionally runs any
//! user-configured site templates ([`agpeer_common::HookSearchSite`]). No site
//! name, indexer, or scraper logic is compiled in: every search scope and site
//! URL comes from user config (settings table / config file).

use agpeer_common::{Error, ExtractStrategy, HookSearchSite, Result};
use regex::Regex;
use reqwest::{Client, Url};
use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::Duration;

/// A magnet hit discovered by the built-in search or by an external hook
/// script (JSON output). Normalized into [`agpeer_common::SearchResult`] by
/// the adapter.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(crate) struct SearchHit {
    #[serde(default)]
    pub(crate) magnet: String,
    #[serde(default)]
    pub(crate) title: Option<String>,
    #[serde(default)]
    pub(crate) size: Option<u64>,
    #[serde(default)]
    pub(crate) seeders: Option<u32>,
    #[serde(default)]
    pub(crate) leechers: Option<u32>,
}

impl SearchHit {
    pub(crate) fn new(magnet: String, title: Option<String>) -> Self {
        Self {
            magnet,
            title,
            size: None,
            seeders: None,
            leechers: None,
        }
    }
}

/// Per-request HTTP timeout. The search is additionally bounded by the
/// backend's configured overall timeout, so one slow site can never hang a
/// search indefinitely.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const USER_AGENT: &str = "Mozilla/5.0 (compatible; agpeer)";

/// Search engines queried in order: DuckDuckGo HTML, DuckDuckGo lite, Bing.
/// Magnet URIs rarely surface raw in ranked engines, so the query is crafted
/// to include the literal `magnet:?xt=urn:btih:` token plus `site:` scopes.
const ENGINES: [&str; 3] = [
    "https://html.duckduckgo.com/html/",
    "https://lite.duckduckgo.com/lite/",
    "https://www.bing.com/search",
];

const MAGNET_PATTERN: &str = r#"magnet:\?xt=urn:btih:[A-Za-z0-9]{32,40}(?:&[^"'\s<>]*)*"#;

fn magnet_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(MAGNET_PATTERN).expect("magnet regex is valid"))
}

/// Deduplication identity for a magnet URI: the lowercased `btih:` info hash,
/// or the whole URI when no hash is present. Two magnets for the same torrent
/// that differ only in trailing parameters (or hash case) collapse to one hit.
pub(crate) fn magnet_key(magnet: &str) -> String {
    if let Some(pos) = magnet.find("btih:") {
        let rest = &magnet[pos + 5..];
        let end = rest.find('&').unwrap_or(rest.len());
        let hash = &rest[..end];
        if !hash.is_empty() {
            return hash.to_ascii_lowercase();
        }
    }
    magnet.to_string()
}

/// Percent-decode for the built-in search. Shares semantics with the torrent
/// crate (and with `SearchHit` display names) via the single implementation in
/// `agpeer_common` — a literal `+` decodes to a space.
pub(crate) use agpeer_common::percent_decode;

/// Percent-encode a string for use in a query parameter.
pub(crate) fn percent_encode(s: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(s.len() * 2);
    for &byte in s.as_bytes() {
        let c = byte as char;
        if byte.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~') {
            out.push(c);
        } else {
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    out
}

/// Minimal HTML entity un-escaping for titles/labels.
fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#039;", "'")
        .replace("&apos;", "'")
        .replace("&#x27;", "'")
}

/// Run a generic engine search for `query`, scoped to the user-configured
/// `domains` via `site:` filters. Returns hits with titles when the engine
/// exposes them.
pub async fn generic_search(
    client: &Client,
    query: &str,
    domains: &[String],
    max_results: usize,
) -> Result<Vec<SearchHit>> {
    let keywords = if domains.is_empty() {
        format!("{query} magnet:?xt=urn:btih:")
    } else {
        let site_scope = domains
            .iter()
            .map(|d| format!("site:{d}"))
            .collect::<Vec<_>>()
            .join(" ");
        format!("{site_scope} {query} magnet:?xt=urn:btih:")
    };

    let mut out: Vec<SearchHit> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for engine in ENGINES {
        let url = match Url::parse_with_params(engine, &[("q", keywords.as_str())]) {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!(engine, error = %e, "built-in search: could not build engine URL");
                continue;
            }
        };
        match fetch_text(client, url.as_str()).await {
            Ok(html) => note_hits(&html, &mut seen, &mut out),
            Err(e) => {
                tracing::warn!(engine, error = %e, "built-in search: engine request failed");
            }
        }
        if out.len() >= max_results {
            break;
        }
    }
    out.truncate(max_results);
    Ok(out)
}

/// Run one user-configured site template search.
pub async fn search_site(
    client: &Client,
    site: &HookSearchSite,
    query: &str,
) -> Result<Vec<SearchHit>> {
    let template = site.search.trim();
    if template.is_empty() {
        return Err(Error::Backend(format!(
            "hook_search.sites entry '{}' has an empty search URL",
            site.domain
        )));
    }
    if !template.contains("{query}") {
        return Err(Error::Backend(format!(
            "hook_search.sites entry '{}' search URL must contain a {{query}} token",
            site.domain
        )));
    }
    let url = Url::parse(&template.replace("{query}", &percent_encode(query))).map_err(|e| {
        Error::Backend(format!(
            "hook_search.sites entry '{}' has an invalid search URL: {e}",
            site.domain
        ))
    })?;

    let page = fetch_text(client, url.as_str()).await?;
    let mut out: Vec<SearchHit> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    match site.extract {
        ExtractStrategy::Table => note_hits(&page, &mut seen, &mut out),
        ExtractStrategy::Detail => {
            let max_pages = site.max_pages.unwrap_or(5).clamp(1, 10);
            detail_hits(client, url.as_str(), &page, max_pages, &mut seen, &mut out).await?;
        }
        ExtractStrategy::Regex => {
            let pattern = site.pattern.clone().ok_or_else(|| {
                Error::Backend(format!(
                    "hook_search.sites entry '{}' uses extract = \"regex\" but has no pattern",
                    site.domain
                ))
            })?;
            for magnet in extract_regex(&page, &pattern)? {
                if seen.insert(magnet_key(&magnet)) {
                    out.push(SearchHit::new(magnet, None));
                }
            }
        }
    }
    Ok(out)
}

/// Follow the top detail-page links on a result page and take each page's
/// first magnet link.
async fn detail_hits(
    client: &Client,
    base: &str,
    html: &str,
    max_pages: usize,
    seen: &mut HashSet<String>,
    out: &mut Vec<SearchHit>,
) -> Result<()> {
    for (url, text) in detail_candidates(html, base, max_pages) {
        let detail = match fetch_text(client, &url).await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(url = %url, error = %e, "built-in search: detail page request failed");
                continue;
            }
        };
        if let Some(first) = magnet_re().find(&detail) {
            let magnet = first.as_str().to_string();
            if seen.insert(magnet_key(&magnet)) {
                out.push(SearchHit::new(
                    magnet,
                    (!text.trim().is_empty()).then_some(text.trim().to_string()),
                ));
            }
        }
    }
    Ok(())
}

/// Collect candidate detail-page links from a result page: **same-origin**
/// absolute or relative hrefs that are not magnets and not anchors. Cross-origin
/// links (absolute, protocol-relative, or redirecting off the template's own
/// host) are rejected so a configured site can never make the core fetch
/// arbitrary internal/LAN URLs.
fn detail_candidates(html: &str, base: &str, max_pages: usize) -> Vec<(String, String)> {
    let Some(base_url) = Url::parse(base).ok() else {
        return Vec::new();
    };
    let base_origin = origin(&base_url);
    let mut out = Vec::new();
    for anchor in scan_anchors(html) {
        if out.len() >= max_pages {
            break;
        }
        let href = anchor.href.trim();
        if href.is_empty()
            || href.starts_with("magnet:")
            || href.starts_with("javascript:")
            || href.starts_with('#')
            || href.starts_with("mailto:")
        {
            continue;
        }
        let Ok(url) = base_url.join(href) else {
            continue;
        };
        // Reject anything that leaves the template's origin.
        if origin(&url) != base_origin {
            continue;
        }
        let resolved = url.to_string();
        if resolved == base {
            continue;
        }
        out.push((resolved, anchor.text));
    }
    out
}

/// `(scheme, host, effective port)` identity for origin comparison.
fn origin(url: &Url) -> (String, Option<String>, Option<u16>) {
    (
        url.scheme().to_string(),
        url.host_str().map(|h| h.to_string()),
        url.port_or_known_default(),
    )
}

/// Apply a user-supplied regex, taking capture group 1 or the whole match as
/// the magnet URI.
fn extract_regex(html: &str, pattern: &str) -> Result<Vec<String>> {
    let re = Regex::new(pattern)
        .map_err(|e| Error::Backend(format!("hook_search.sites has an invalid regex: {e}")))?;
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for caps in re.captures_iter(html) {
        let m = caps
            .get(1)
            .or_else(|| caps.get(0))
            .map(|m| m.as_str())
            .unwrap_or_default();
        if m.starts_with("magnet:") && seen.insert(m.to_string()) {
            out.push(m.to_string());
        }
    }
    Ok(out)
}

/// Collect magnet hits from a page: direct `<a href="magnet:...">` anchors
/// (with link text as the title), `uddg=`-wrapped redirect targets (DuckDuckGo
/// html/lite), and any bare `magnet:` tokens in the raw page. `seen` holds
/// [`magnet_key`] identities.
fn note_hits(html: &str, seen: &mut HashSet<String>, out: &mut Vec<SearchHit>) {
    for anchor in scan_anchors(html) {
        if let Some(magnet) = resolved_magnet(&anchor.href) {
            if seen.insert(magnet_key(&magnet)) {
                let title = if anchor.text.trim().is_empty() {
                    None
                } else {
                    Some(anchor.text.trim().to_string())
                };
                out.push(SearchHit::new(magnet, title));
            }
        }
    }
    for magnet in raw_magnets(html) {
        if seen.insert(magnet_key(&magnet)) {
            out.push(SearchHit::new(magnet, None));
        }
    }
}

/// The magnet inside an anchor href, decoding `uddg=` redirect wrappers when
/// present (the raw string may itself be a DuckDuckGo redirect link).
fn resolved_magnet(href: &str) -> Option<String> {
    if let Some(m) = magnet_re().find(href) {
        return Some(m.as_str().to_string());
    }
    if href.contains("uddg=") {
        for target in uddg_targets(href) {
            if let Some(m) = magnet_re().find(&target) {
                return Some(m.as_str().to_string());
            }
        }
    }
    None
}

/// All raw `magnet:` tokens appearing anywhere in the page.
fn raw_magnets(html: &str) -> Vec<String> {
    magnet_re()
        .find_iter(html)
        .map(|m| m.as_str().to_string())
        .collect()
}

/// Decode every `uddg=` query value in `s` (used to unwrap DuckDuckGo
/// redirect links into their real targets).
fn uddg_targets(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(pos) = rest.find("uddg=") {
        let value = &rest[pos + 5..];
        let end = value
            .find(['&', '"', '\'', '<', ' ', '\n', '\r'])
            .unwrap_or(value.len());
        if end > 0 {
            out.push(percent_decode(&value[..end]));
        }
        rest = &rest[pos + 5 + end.max(1)..];
    }
    out
}

/// A parsed `<a href="...">text</a>` element.
struct Anchor {
    href: String,
    text: String,
}

/// Extract all `<a ...>...</a>` anchors in document order, with HTML entities
/// in href/text un-escaped.
fn scan_anchors(html: &str) -> Vec<Anchor> {
    let mut out = Vec::new();
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let is_anchor = bytes[i] == b'<'
            && bytes.get(i + 1) == Some(&b'a')
            && matches!(bytes.get(i + 2), Some(b' ') | Some(b'>'));
        if is_anchor {
            if let Some(rel) = bytes[i..].iter().position(|&c| c == b'>') {
                let tag = &html[i..i + rel];
                let href = attr_value(tag, "href").unwrap_or_default();
                let text_start = i + rel + 1;
                let rest = &bytes[text_start..];
                if let Some(end) = rest.windows(4).position(|w| w == b"</a>") {
                    let text = html[text_start..text_start + end]
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ");
                    out.push(Anchor {
                        href: html_unescape(&href),
                        text: html_unescape(&text),
                    });
                    i = text_start + end + 4;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

/// Read the value of an HTML attribute, e.g. `href="..."` or `href='...'`.
fn attr_value(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=");
    let rest = tag.split_once(&needle)?.1.trim_start();
    let quote = rest.chars().next()?;
    if quote == '"' || quote == '\'' {
        let value = &rest[quote.len_utf8()..];
        let end = value.find(quote)?;
        Some(value[..end].to_string())
    } else {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        Some(rest[..end].to_string())
    }
}

/// Fetch a page as lossy UTF-8 text. Non-2xx responses and network errors are
/// treated as empty/enabled-skip outcomes (best-effort engine search).
async fn fetch_text(client: &Client, url_text: &str) -> Result<String> {
    let url =
        Url::parse(url_text).map_err(|e| Error::Backend(format!("invalid URL {url_text}: {e}")))?;
    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .await
        .map_err(|e| Error::Backend(format!("request failed for {url_text}: {e}")))?;
    if !response.status().is_success() {
        return Ok(String::new());
    }
    let body = response
        .bytes()
        .await
        .map_err(|e| Error::Backend(format!("response read failed for {url_text}: {e}")))?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// Build a shared HTTP client with a bounded per-request timeout for the
/// built-in search.
///
/// Redirects are followed only while they stay on the same host: a configured
/// site (or a page embedded in one) can never bounce the core's requests onto
/// internal/LAN/cloud-metadata hosts.
pub(crate) fn http_client() -> Result<Client> {
    Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            // `previous()` is the history of already-followed URLs; keep
            // following only while the next hop stays on the same host.
            let previous = attempt
                .previous()
                .last()
                .and_then(|u| u.host_str())
                .unwrap_or_default();
            let next = attempt.url().host_str().unwrap_or_default();
            if !next.is_empty() && previous != next {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|e| Error::Backend(format!("failed to build HTTP client: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAGNET_A: &str = "magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const MAGNET_B: &str = "magnet:?xt=urn:btih:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const MAGNET_A_HASH_UPPER: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    #[test]
    fn raw_magnet_scan_finds_tokens_in_attributes_and_text() {
        let html = format!("<a href=\"{MAGNET_A}&dn=Foo\">Foo</a> plain text {MAGNET_B}");
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        note_hits(&html, &mut seen, &mut out);
        let magnets: Vec<String> = out.iter().map(|h| h.magnet.clone()).collect();
        assert!(magnets.contains(&format!("{MAGNET_A}&dn=Foo")));
        assert!(magnets.contains(&MAGNET_B.to_string()));
        // Deduplication: same magnet twice yields one hit.
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn anchor_title_becomes_hit_title() {
        let html = format!("<a href=\"{MAGNET_A}\">My &amp; File</a>");
        let hits = generic_extract(&html);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title.as_deref(), Some("My & File"));
        assert_eq!(hits[0].magnet, MAGNET_A);
    }

    #[test]
    fn uddg_redirect_targets_are_decoded() {
        let encoded = percent_encode(MAGNET_A);
        let html = format!(
            "<a rel=\"nofollow\" href=\"//duckduckgo.com/l/?uddg={encoded}&rut=abc\">x</a>"
        );
        let hits = generic_extract(&html);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].magnet, MAGNET_A);
    }

    #[test]
    fn percent_encoding_roundtrips() {
        let original = MAGNET_A;
        assert_eq!(percent_decode(&percent_encode(original)), original);
        assert_eq!(percent_decode("a%20b%2Fc"), "a b/c");
    }

    #[test]
    fn detail_candidates_skip_magnets_and_anchors() {
        let html = format!(
            "<a href=\"{MAGNET_A}\">direct</a><a href=\"/view/1\">one</a><a href=\"#section\">in-page</a><a href=\"javascript:void(0)\">js</a><a href=\"https://nyaa.example/view/2\">two</a>"
        );
        let candidates = detail_candidates(&html, "https://nyaa.example/?q=x", 10);
        let urls: Vec<String> = candidates.iter().map(|(u, _)| u.clone()).collect();
        assert_eq!(
            urls,
            vec![
                "https://nyaa.example/view/1".to_string(),
                "https://nyaa.example/view/2".to_string(),
            ]
        );
    }

    #[test]
    fn detail_candidates_reject_cross_origin_and_protocol_relative_links() {
        let base = "https://index.example/search?q=x";
        let html = "<a href=\"/ok\">ok</a><a href=\"https://other.example/x\">other</a><a href=\"//other.example/y\">proto-relative</a><a href=\"http://169.254.169.254/latest/meta-data\">metadata</a>";
        let candidates = detail_candidates(html, base, 10);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, "https://index.example/ok");
    }

    #[test]
    fn regex_strategy_extracts_group_1_and_matches() {
        let html = format!("<x>{MAGNET_A}</x> <y>{MAGNET_B}</y>");
        let magnets = extract_regex(&html, r#"magnet:\?xt=urn:btih:[a-f0-9]+"#).unwrap();
        assert_eq!(magnets, vec![MAGNET_A.to_string(), MAGNET_B.to_string()]);

        // Prefix/suffix patterns using capture group 1.
        let grouped = extract_regex(&html, r#"<x>(magnet:[^<]+)</x>"#).unwrap();
        assert_eq!(grouped, vec![MAGNET_A.to_string()]);
    }

    #[test]
    fn invalid_regex_is_a_typed_error() {
        let err = extract_regex("<html/>", "(unclosed").unwrap_err();
        assert!(matches!(err, agpeer_common::Error::Backend(_)));
    }

    #[tokio::test]
    async fn site_template_requires_query_token() {
        let site = HookSearchSite {
            domain: "example".into(),
            search: "https://example.example/search".into(),
            ..Default::default()
        };
        let client = http_client().unwrap();
        let err = search_site(&client, &site, "q").await.unwrap_err();
        assert!(err.to_string().contains("{query}"));
    }

    #[test]
    fn scan_anchors_handles_single_and_double_quotes() {
        let html = "<a href=\"/x\">one</a><a href='/y'>two</a>";
        let anchors = scan_anchors(html);
        assert_eq!(anchors.len(), 2);
        assert_eq!(anchors[0].href, "/x");
        assert_eq!(anchors[0].text, "one");
        assert_eq!(anchors[1].href, "/y");
        assert_eq!(anchors[1].text, "two");
    }

    #[test]
    fn scan_anchors_skips_non_anchor_tags() {
        let html = "<article>x</article><a href=\"/z\">three</a>";
        let anchors = scan_anchors(html);
        assert_eq!(anchors.len(), 1);
        assert_eq!(anchors[0].href, "/z");
    }

    #[test]
    fn scan_anchors_unescapes_entities() {
        let anchors = scan_anchors("<a href=\"/x\">a &amp; b</a>");
        assert_eq!(anchors[0].text, "a & b");
    }

    #[test]
    fn dedupe_collapses_same_hash_with_different_params_or_case() {
        let upper = format!("magnet:?xt=urn:btih:{}", MAGNET_A_HASH_UPPER);
        let html = format!(
            "<a href=\"{MAGNET_A}&dn=One\">one</a><a href=\"{upper}\">two</a><a href=\"{MAGNET_B}\">three</a>"
        );
        let hits = generic_extract(&html);
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn magnet_key_extracts_lowercased_btih() {
        assert_eq!(magnet_key("magnet:?xt=urn:btih:ABCDEF&dn=x"), "abcdef");
        assert_eq!(magnet_key(MAGNET_A), "a".repeat(40));
    }

    fn generic_extract(html: &str) -> Vec<SearchHit> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        note_hits(html, &mut seen, &mut out);
        out
    }
}
