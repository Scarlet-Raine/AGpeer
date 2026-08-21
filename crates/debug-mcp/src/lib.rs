//! `agpeer-debug-mcp` — a fast, low-token debugging MCP server.
//!
//! Coding agents use this to inspect logs and source and to get git summaries
//! without dumping whole files or whole log streams (which would burn tokens).
//! Every tool returns **counts and trimmed, context-limited snippets**, never
//! the raw output of a large command.
//!
//! Two roots matter:
//! - the repository root (`--root`), for source/git tools;
//! - a log directory (`--log-dir`), for the agpeer core's logs.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::schemars::JsonSchema;
use rmcp::{tool, tool_router, ErrorData};
use serde::Deserialize;

/// Hard cap on the number of result lines / matches surfaced per call.
const MAX_RESULTS: usize = 100;
/// Hard cap on characters in a single tool response (keeps token cost low).
const MAX_CHARS: usize = 32 * 1024;
/// Default tail window for `log_tail`.
const DEFAULT_TAIL: usize = 200;
const LOG_LINES_PER_FILE: usize = 2_000;
const LOG_FILE_RETENTION: usize = 20;

/// Directories never walked for code search (build/test artifacts).
const EXCLUDED_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    "dist",
    "build",
    ".next",
    ".turbo",
    "coverage",
    "vendor",
    ".cache",
];

/// Error type shared by the debug helpers.
#[derive(Debug, thiserror::Error)]
pub enum DebugError {
    #[error("io error on {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid regex `{pat}`: {msg}")]
    Regex { pat: String, msg: String },
    #[error("{0}")]
    Other(String),
}

fn err_to_data(e: DebugError) -> ErrorData {
    ErrorData::internal_error(e.to_string(), None)
}

fn io_err(path: &Path, e: std::io::Error) -> DebugError {
    DebugError::Io {
        path: path.display().to_string(),
        source: e,
    }
}

/// The server state: where to look for code and logs.
#[derive(Clone)]
pub struct DebugServer {
    root: PathBuf,
    log_dir: PathBuf,
}

impl DebugServer {
    pub fn new(root: PathBuf, log_dir: PathBuf) -> Self {
        Self { root, log_dir }
    }
}

/// True if a relative directory should be skipped during walking.
fn is_excluded_dir(name: &str) -> bool {
    EXCLUDED_DIRS.contains(&name)
}

/// Apply a character cap, appending a truncation marker when tripped.
fn cap(mut s: String) -> String {
    if s.len() > MAX_CHARS {
        s.truncate(MAX_CHARS);
        s.push_str("\n… (output truncated)");
    }
    s
}

/// List log files (newest first) under the log dir.
fn log_files(log_dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(log_dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .map(|n| n.to_string_lossy().starts_with("agpeer"))
                    .unwrap_or(false)
        })
        .collect();
    files.sort_by_key(|p| mtime(p));
    files.reverse();
    files
}

fn mtime(p: &Path) -> std::time::SystemTime {
    p.metadata()
        .and_then(|m| m.modified())
        .unwrap_or(UNIX_EPOCH)
}

fn read_lines(p: &Path) -> Result<Vec<String>, DebugError> {
    let contents = std::fs::read_to_string(p).map_err(|e| io_err(p, e))?;
    Ok(contents.lines().map(|s| s.to_string()).collect())
}

fn rel(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
        .replace('\\', "/")
}

// ---------------------------------------------------------------------------
// Tool inputs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, JsonSchema, Default)]
pub struct LogTailInput {
    /// Number of last lines to inspect (default 200, max 2000).
    #[serde(default)]
    pub lines: Option<usize>,
    /// Keep only lines containing this substring (case-insensitive).
    #[serde(default)]
    pub contains: Option<String>,
    /// Keep only lines at this level (ERROR, WARN, INFO, DEBUG, TRACE).
    #[serde(default)]
    pub level: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, Default)]
pub struct LogSearchInput {
    /// Regex to match against log lines.
    pub pattern: String,
    /// Lines of surrounding context per match (default 0, max 5).
    #[serde(default)]
    pub context: Option<usize>,
    /// Maximum matches to return (default 100).
    #[serde(default)]
    pub limit: Option<usize>,
    /// Restrict to a substring/level filter first (optional).
    #[serde(default)]
    pub contains: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, Default)]
pub struct GrepInput {
    pub pattern: String,
    #[serde(default)]
    pub include: Option<String>,
    #[serde(default)]
    pub context: Option<usize>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub case_insensitive: bool,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, Default)]
pub struct ReadInput {
    /// Path relative to the repo root.
    pub path: String,
    /// 1-based start line (inclusive).
    #[serde(default)]
    pub start: Option<usize>,
    /// Number of lines to read.
    #[serde(default)]
    pub lines: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, Default)]
pub struct FilesInput {
    /// Optional regex matched against relative paths.
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, Default)]
pub struct SymbolInput {
    pub name: String,
    /// File extension to restrict to (e.g. "rs"); optional.
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, Default)]
pub struct GitLogInput {
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, Default)]
pub struct GitDiffInput {
    /// Restrict diff to this path.
    #[serde(default)]
    pub path: Option<String>,
    /// Include a full diff (not just --stat).
    #[serde(default)]
    pub full: bool,
}

// ---------------------------------------------------------------------------
// MCP server
// ---------------------------------------------------------------------------

#[tool_router(server_handler)]
impl DebugServer {
    /// Point to the roots in use: repo root, log dir, latest log file.
    #[tool(description = "Report active roots: repo, log dir, latest log file, sizes.")]
    async fn runtime_info(&self) -> Result<String, ErrorData> {
        let latest = log_files(&self.log_dir).into_iter().next();
        let mut out = String::new();
        let _ = writeln!(out, "root:     {}", self.root.display());
        let _ = writeln!(out, "log_dir:  {}", self.log_dir.display());
        let _ = writeln!(
            out,
            "log_rotation: {} lines/file, {} files retained",
            LOG_LINES_PER_FILE, LOG_FILE_RETENTION
        );
        if let Some(l) = &latest {
            let size = l.metadata().map(|m| m.len()).unwrap_or(0);
            let _ = writeln!(out, "latest_log: {} ({} bytes)", l.display(), size);
        } else {
            let _ = writeln!(out, "latest_log: (none found)");
        }
        Ok(cap(out))
    }

    /// Tail the most recent agpeer log file, filtered and capped.
    #[tool(description = "Tail the latest agpeer log file with optional substring/level filter.")]
    async fn log_tail(&self, Parameters(p): Parameters<LogTailInput>) -> Result<String, ErrorData> {
        let Some(latest) = log_files(&self.log_dir).into_iter().next() else {
            return Ok("no agpeer log files found".to_string());
        };
        let lines = read_lines(&latest).map_err(err_to_data)?;
        let want = p.lines.unwrap_or(DEFAULT_TAIL).clamp(1, 2000);
        let contains = p.contains.as_deref().map(|s| s.to_lowercase());
        let level = p.level.as_deref().map(|s| s.to_uppercase());
        let mut out = String::new();
        let _ = writeln!(
            out,
            "# {} ({} lines)",
            rel(&latest, &self.log_dir),
            lines.len()
        );
        let mut shown = 0usize;
        let start = lines.len().saturating_sub(want);
        for line in lines.iter().skip(start) {
            if shown >= MAX_RESULTS {
                break;
            }
            if let Some(c) = &contains {
                if !line.to_lowercase().contains(c) {
                    continue;
                }
            }
            if let Some(lv) = &level {
                // " LEVEL " is the standard tracing console shape.
                if !line.contains(&format!(" {lv} ")) {
                    continue;
                }
            }
            let _ = writeln!(out, "{}", line);
            shown += 1;
        }
        let _ = writeln!(out, "-- {} matching lines (capped)", shown);
        Ok(cap(out))
    }

    /// Search all agpeer log files for a regex, with context and a cap.
    #[tool(description = "Regex-search all agpeer log files; returns capped matches with context.")]
    async fn log_search(
        &self,
        Parameters(p): Parameters<LogSearchInput>,
    ) -> Result<String, ErrorData> {
        let re = match regex::RegexBuilder::new(&p.pattern)
            .case_insensitive(true)
            .build()
        {
            Ok(r) => r,
            Err(e) => {
                return Err(err_to_data(DebugError::Regex {
                    pat: p.pattern,
                    msg: e.to_string(),
                }))
            }
        };
        let ctx = p.context.unwrap_or(0).clamp(0, 5);
        let limit = p.limit.unwrap_or(MAX_RESULTS).clamp(1, MAX_RESULTS);
        let contains = p.contains.as_deref().map(|s| s.to_lowercase());
        let files = log_files(&self.log_dir);
        let mut out = String::new();
        let mut total = 0usize;
        let mut shown = 0usize;
        'outer: for file in &files {
            let Ok(lines) = read_lines(file) else {
                continue;
            };
            for (idx, line) in lines.iter().enumerate() {
                if let Some(c) = &contains {
                    if !line.to_lowercase().contains(c) {
                        continue;
                    }
                }
                if re.is_match(line) {
                    total += 1;
                    if shown >= limit {
                        break 'outer;
                    }
                    let lo = idx.saturating_sub(ctx);
                    let hi = (idx + 1 + ctx).min(lines.len());
                    let _ = writeln!(out, "== {}:{idx} ==", rel(file, &self.log_dir));
                    for (k, l) in lines[lo..hi].iter().enumerate() {
                        let lineno = lo + k + 1;
                        let marker = if lo + k == idx { ">" } else { " " };
                        let _ = writeln!(out, "{marker}{lineno}: {l}");
                    }
                    shown += 1;
                }
            }
        }
        let _ = writeln!(
            out,
            "-- {} matches across {} file(s), {} shown",
            total,
            files.len(),
            shown
        );
        Ok(cap(out))
    }

    /// Regex-search source under the repo root, capped, with counts.
    #[tool(
        description = "Regex-search project source (skips target/node_modules/.git). Returns capped matches."
    )]
    async fn code_grep(&self, Parameters(p): Parameters<GrepInput>) -> Result<String, ErrorData> {
        let re = match regex::RegexBuilder::new(&p.pattern)
            .case_insensitive(p.case_insensitive)
            .build()
        {
            Ok(r) => r,
            Err(e) => {
                return Err(err_to_data(DebugError::Regex {
                    pat: p.pattern,
                    msg: e.to_string(),
                }))
            }
        };
        let include = match p.include.as_deref() {
            Some(pat) => Some(regex::Regex::new(pat).map_err(|e| {
                err_to_data(DebugError::Regex {
                    pat: pat.to_string(),
                    msg: e.to_string(),
                })
            })?),
            None => None,
        };
        let ctx = p.context.unwrap_or(0).clamp(0, 5);
        let limit = p.limit.unwrap_or(MAX_RESULTS).clamp(1, MAX_RESULTS);
        let walker = Walker::new(&self.root, include.as_ref());
        let mut out = String::new();
        let mut total = 0usize;
        let mut shown = 0usize;
        for (file, lines) in walker.by_lines() {
            for (idx, line) in lines.iter().enumerate() {
                if re.is_match(line) {
                    total += 1;
                    if shown >= limit {
                        let _ = writeln!(out, "-- stopped at {} total matches", total);
                        return Ok(cap(out));
                    }
                    let lo = idx.saturating_sub(ctx);
                    let hi = (idx + 1 + ctx).min(lines.len());
                    let _ = writeln!(out, "== {}:{idx} ==", rel(&file, &self.root));
                    for (k, l) in lines[lo..hi].iter().enumerate() {
                        let lineno = lo + k + 1;
                        let marker = if lo + k == idx { ">" } else { " " };
                        let _ = writeln!(out, "{marker}{lineno}: {l}");
                    }
                    shown += 1;
                }
            }
        }
        let _ = writeln!(out, "-- {} total matches, {} shown", total, shown);
        Ok(cap(out))
    }

    /// Read a file (or a line range) from the repo.
    #[tool(description = "Read a repo file, optionally a line range; capped for tokens.")]
    async fn code_read(&self, Parameters(p): Parameters<ReadInput>) -> Result<String, ErrorData> {
        let path = canonical_within(&self.root, &p.path).map_err(err_to_data)?;
        if !path.is_file() {
            return Err(err_to_data(DebugError::Other(format!(
                "not a file: {}",
                p.path
            ))));
        }
        let lines = read_lines(&path).map_err(err_to_data)?;
        let start = p.start.unwrap_or(1).max(1);
        let count = p.lines.unwrap_or(lines.len()).clamp(1, 1000);
        let mut out = String::new();
        let _ = writeln!(out, "# {}", rel(&path, &self.root));
        for i in start..(start + count).min(lines.len() + 1) {
            let _ = writeln!(out, "{i}: {}", lines[i - 1]);
        }
        Ok(cap(out))
    }

    /// List files under the repo root matching a pattern.
    #[tool(description = "List repo files (skips build dirs), optional path regex; capped.")]
    async fn code_files(&self, Parameters(p): Parameters<FilesInput>) -> Result<String, ErrorData> {
        let pat = match p.pattern.as_deref() {
            Some(pat) if !pat.is_empty() => Some(regex::Regex::new(pat).map_err(|e| {
                err_to_data(DebugError::Regex {
                    pat: pat.to_string(),
                    msg: e.to_string(),
                })
            })?),
            _ => None,
        };
        let limit = p.limit.unwrap_or(MAX_RESULTS).clamp(1, MAX_RESULTS);
        let walker = Walker::new(&self.root, None);
        let mut out = String::new();
        let mut total = 0usize;
        for file in walker.files() {
            total += 1;
            let r = rel(file, &self.root);
            if let Some(pat) = &pat {
                if !pat.is_match(&r) {
                    continue;
                }
            }
            if out.lines().count() >= limit {
                break;
            }
            let _ = writeln!(out, "{r}");
        }
        let _ = writeln!(
            out,
            "-- {} files indexed, {} listed",
            total,
            out.lines().count().saturating_sub(1)
        );
        Ok(cap(out))
    }

    /// Find a definition (fn/struct/enum/trait/impl/const/etc.) by name.
    #[tool(
        description = "Find a symbol definition by name across the repo; returns file:line + snippet."
    )]
    async fn code_symbol(
        &self,
        Parameters(p): Parameters<SymbolInput>,
    ) -> Result<String, ErrorData> {
        let name = p.name.trim();
        if name.is_empty() {
            return Err(err_to_data(DebugError::Other(
                "symbol name is required".to_string(),
            )));
        }
        let lang = p.lang.as_deref().map(|s| s.to_lowercase());
        let limit = p.limit.unwrap_or(MAX_RESULTS).clamp(1, MAX_RESULTS);
        // Definition keyword followed by the name as a whole identifier.
        let def_re = regex::Regex::new(&format!(
            r"\b(?:pub\s+)?(?:async\s+)?(?:unsafe\s+)?(?:fn|struct|enum|trait|mod|const|static|type)\s+{name}\b|\bimpl(?:<[^>]*>)?\s+[\w:<>, ]*\b{name}\b"
        ))
        .map_err(|e| {
            err_to_data(DebugError::Regex {
                pat: format!("definition for {name}"),
                msg: e.to_string(),
            })
        })?;
        let walker = Walker::new(&self.root, None);
        let mut out = String::new();
        let mut total = 0usize;
        'outer: for (file, lines) in walker.by_lines() {
            let relp = rel(&file, &self.root);
            if let Some(lang) = lang.as_deref() {
                if !relp.ends_with(&format!(".{lang}")) {
                    continue;
                }
            }
            for (idx, line) in lines.iter().enumerate() {
                if def_re.is_match(line) {
                    total += 1;
                    if out.lines().count() >= limit {
                        break 'outer;
                    }
                    let lo = idx.saturating_sub(2);
                    let hi = (idx + 3).min(lines.len());
                    let _ = writeln!(out, "== {relp}:{} ==", idx + 1);
                    for (k, l) in lines[lo..hi].iter().enumerate() {
                        let _ = writeln!(out, "{:>5}: {l}", lo + k + 1);
                    }
                }
            }
        }
        let _ = writeln!(out, "-- {} definition sites (capped)", total);
        Ok(cap(out))
    }

    /// Git working-tree status + last commit, concise.
    #[tool(description = "Git status summary: branch, changes, untracked files (capped).")]
    async fn git_status(&self) -> Result<String, ErrorData> {
        let branch = run_git(&self.root, &["rev-parse", "--abbrev-ref", "HEAD"])
            .await
            .unwrap_or_else(|_| "(no branch)".into());
        let short = run_git(&self.root, &["status", "--short"])
            .await
            .unwrap_or_default();
        let last = run_git(&self.root, &["log", "-1", "--oneline"])
            .await
            .unwrap_or_default();
        let mut out = String::new();
        let _ = writeln!(out, "branch: {branch}");
        let _ = writeln!(out, "last:   {last}");
        let _ = writeln!(out, "--- status --short ({}) ---", short.lines().count());
        let _ = out.write_str(&short);
        Ok(cap(out))
    }

    /// Recent commit log (oneline).
    #[tool(description = "Recent commit log, oneline format.")]
    async fn git_log(&self, Parameters(p): Parameters<GitLogInput>) -> Result<String, ErrorData> {
        let n = p.limit.unwrap_or(20).clamp(1, 100);
        match run_git(&self.root, &["log", "--oneline", &format!("-{n}")]).await {
            Ok(log) => Ok(cap(log)),
            Err(e) => Err(err_to_data(e)),
        }
    }

    /// Diff stat (and optionally full diff) of the working tree.
    #[tool(description = "Git diff summary (--stat) or full diff, capped for tokens.")]
    async fn git_diff(&self, Parameters(p): Parameters<GitDiffInput>) -> Result<String, ErrorData> {
        let mut args: Vec<&str> = vec!["diff"];
        if let Some(path) = &p.path {
            args.push("--");
            args.push(path);
        } else if !p.full {
            args.push("--stat");
        }
        match run_git(&self.root, &args).await {
            Ok(diff) => Ok(cap(diff)),
            Err(e) => Err(err_to_data(e)),
        }
    }
}

async fn run_git(root: &Path, args: &[&str]) -> Result<String, DebugError> {
    let out = tokio::process::Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .await
        .map_err(|e| io_err(root, e))?;
    if !out.status.success() {
        return Err(DebugError::Other(format!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Resolve a user path and ensure it stays within `root`.
fn canonical_within(root: &Path, input: &str) -> Result<PathBuf, DebugError> {
    let root_canon = root.canonicalize().map_err(|e| DebugError::Io {
        path: root.display().to_string(),
        source: e,
    })?;
    let candidate = if Path::new(input).is_absolute() {
        PathBuf::from(input)
    } else {
        root_canon.join(input)
    };
    let canon = candidate.canonicalize().map_err(|e| DebugError::Io {
        path: candidate.display().to_string(),
        source: e,
    })?;
    if !canon.starts_with(&root_canon) {
        return Err(DebugError::Other(format!(
            "path escapes repo root: {input}"
        )));
    }
    Ok(canon)
}

/// Depth-first walker over the repo root that skips build/artifact dirs and
/// yields text files.
struct Walker<'a> {
    root: &'a Path,
    include: Option<&'a regex::Regex>,
    files: Vec<PathBuf>,
}

impl<'a> Walker<'a> {
    fn new(root: &'a Path, include: Option<&'a regex::Regex>) -> Self {
        let mut this = Walker {
            root,
            include,
            files: Vec::new(),
        };
        if root.is_dir() {
            this.collect(root);
        }
        this
    }

    fn collect(&mut self, dir: &Path) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if !is_excluded_dir(&path.file_name().unwrap_or_default().to_string_lossy()) {
                    self.collect(&path);
                }
            } else if path.is_file() {
                if let Some(re) = self.include {
                    let r = rel(&path, self.root);
                    if !re.is_match(&r) {
                        continue;
                    }
                }
                self.files.push(path);
            }
        }
    }

    fn files(&self) -> impl Iterator<Item = &PathBuf> {
        self.files.iter()
    }

    /// Iterate files as `(path, Vec<lines>)`, lazily reading each file.
    fn by_lines(self) -> Vec<(PathBuf, Vec<String>)> {
        self.files
            .into_iter()
            .filter_map(|f| read_lines(&f).ok().map(|l| (f, l)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "agpeer-debug-mcp-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn canonical_within_rejects_escape() {
        let root = tmpdir("root");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let inner = canonical_within(&root, "sub").unwrap();
        assert!(inner.starts_with(root.canonicalize().unwrap()));
        assert!(canonical_within(&root, "../outside").is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn cap_truncates_large_output() {
        let big = "x".repeat(MAX_CHARS + 100);
        let capped = cap(big.clone());
        assert!(capped.contains("truncated"));
        assert!(capped.len() < big.len());
    }

    #[test]
    fn walker_skips_build_dirs() {
        let root = tmpdir("walk");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(root.join("target/artifact.rs"), "fn x() {}\n").unwrap();
        let w = Walker::new(&root, None);
        let files: Vec<_> = w.files().map(|p| rel(p, &root)).collect();
        assert!(files.iter().any(|f| f == "src/main.rs"));
        assert!(!files.iter().any(|f| f == "target/artifact.rs"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn log_files_lists_newest_first() {
        let dir = tmpdir("logs");
        std::fs::write(dir.join("agpeer.log.2026-08-18"), "old\n").unwrap();
        std::fs::write(dir.join("agpeer.log.2026-08-19"), "new\n").unwrap();
        std::fs::write(dir.join("unrelated.txt"), "x\n").unwrap();
        let files = log_files(&dir);
        assert_eq!(files.len(), 2);
        assert!(files[0].to_string_lossy().contains("2026-08-19"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn is_excluded_dir_covers_artifacts() {
        assert!(is_excluded_dir("target"));
        assert!(is_excluded_dir(".git"));
        assert!(is_excluded_dir("node_modules"));
        assert!(!is_excluded_dir("src"));
    }

    #[test]
    fn tool_router_exposes_expected_tools() {
        let router = DebugServer::tool_router();
        let names: Vec<String> = router
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        for expected in [
            "runtime_info",
            "log_tail",
            "log_search",
            "code_grep",
            "code_read",
            "code_files",
            "code_symbol",
            "git_status",
            "git_log",
            "git_diff",
        ] {
            assert!(
                names.iter().any(|n| n == expected),
                "tool registry missing {expected}; got {names:?}"
            );
        }
    }
}
