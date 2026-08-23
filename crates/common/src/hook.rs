//! Magnet-search site templates.
//!
//! These are **user-configured** descriptions of how to run a magnet search
//! against a specific site. They ship in config/env and runtime settings; the
//! binary itself contains no site-specific search logic. The extraction
//! strategies are generic layout rules ("rows contain direct magnet links",
//! "follow detail pages", "apply this regex") with no compiled-in domains.

use serde::{Deserialize, Serialize};

/// How a magnet-search site template extracts magnet links from fetched pages.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtractStrategy {
    /// Result rows contain direct `magnet:` link hrefs (nyaa-style tables).
    #[default]
    Table,
    /// Follow the top result-page links to detail pages and take each page's
    /// first magnet link (1337x-style).
    Detail,
    /// Apply the user-supplied regex to the page and take group 1 (or the
    /// whole match) as magnet URIs (anything else).
    Regex,
}

/// One user-configured magnet-search site template.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HookSearchSite {
    /// Identifier/display name, e.g. `nyaa`.
    pub domain: String,
    /// Search URL template with `{query}` substituted at search time, e.g.
    /// `https://nyaa.example/?f=0&c=0_0&q={query}`. The domain uses `example`
    /// here; real site URLs are user config, never compiled.
    pub search: String,
    /// How magnets are extracted from the fetched pages.
    pub extract: ExtractStrategy,
    /// For [`ExtractStrategy::Detail`]: how many detail pages to follow
    /// (defaults to 5, bounded at 10).
    pub max_pages: Option<usize>,
    /// For [`ExtractStrategy::Regex`]: the literal regex to apply.
    pub pattern: Option<String>,
}
