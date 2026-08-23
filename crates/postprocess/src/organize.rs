//! Conservative media organization for Jellyfin/Plex-compatible library trees.
//!
//! All completed downloads are moved under a configured library root
//! (`E:\Media` by default in the runtime config) as:
//!
//! ```text
//! <root>/Movies/<Title> (<Year>)/<file>
//! <root>/TV Shows/<Title>/Season NN/<file>
//! <root>/Music/<Artist>/<Album>/<file>
//! <root>/Pictures/<file>
//! <root>/Archives/<file>
//! <root>/Documents/<file>
//! <root>/Software/<file>
//! <root>/Other/<file>
//! ```
//!
//! Heuristics are deliberately conservative: titles are cleaned (dots and
//! underscores become spaces, common release tags are dropped) but files are
//! never renamed, only relocated. When the media kind cannot be determined
//! confidently, the file is placed in the category root without further
//! nesting. Nothing outside the library root is ever touched.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use agpeer_common::Error;

use crate::classify::Category;

/// How a media file should be nested under the library root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaKind {
    /// A single film: `<movies>/<Title> (<Year>)/`.
    Movie { year: Option<u32> },
    /// An episode of a series: `<tv>/<Title>/Season NN/`.
    TvSeries { season: Option<u32> },
    /// A music track: `<music>/<Artist>/<Album>/`.
    Music {
        artist: Option<String>,
        album: Option<String>,
    },
    /// Not confidently identifiable: place at the category root.
    Other,
}

/// Caller-supplied routing hints for a single file (from transfer metadata).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrganizeHints {
    /// Force the media routing regardless of filename heuristics.
    pub media: Option<MediaHint>,
    /// Explicit subpath under the music root, e.g. `Artist/Album`. Path
    /// traversal components are rejected; an invalid subpath is ignored.
    pub music_path: Option<String>,
}

/// Explicit media routing requested via transfer metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaHint {
    /// Route as a series episode even without an `SxxEyy` marker.
    Tv,
    /// Route as a film even when no year is present.
    Movie,
    /// Route into the anime tree instead of the general TV tree.
    Anime,
}

/// Computes destinations under the library root for a given source file.
#[derive(Debug, Clone)]
pub struct Organizer {
    pub library_root: PathBuf,
    /// Category → subfolder name under the library root.
    pub category_folders: HashMap<Category, String>,
    /// Subfolder for series episodes (default `TV Shows`).
    pub tv_folder: String,
    /// Subfolder for films (default `Movies`).
    pub movies_folder: String,
    /// Subfolder for anime; when `None`, anime episodes go to [`Self::tv_folder`].
    pub anime_folder: Option<String>,
}

impl Organizer {
    /// Create an organizer with the default Jellyfin-friendly category names.
    pub fn new(library_root: PathBuf) -> Self {
        let mut category_folders = HashMap::new();
        category_folders.insert(Category::Audio, "Music".to_string());
        category_folders.insert(Category::Video, "Movies".to_string());
        category_folders.insert(Category::Image, "Pictures".to_string());
        category_folders.insert(Category::Archive, "Archives".to_string());
        category_folders.insert(Category::Document, "Documents".to_string());
        category_folders.insert(Category::Software, "Software".to_string());
        category_folders.insert(Category::Unknown, "Other".to_string());
        Self {
            library_root,
            category_folders,
            tv_folder: "TV Shows".to_string(),
            movies_folder: "Movies".to_string(),
            anime_folder: None,
        }
    }

    /// Override the subfolder used for series episodes.
    pub fn with_tv_folder(mut self, folder: impl Into<String>) -> Self {
        self.tv_folder = folder.into();
        self
    }

    /// Override the subfolder used for films.
    pub fn with_movies_folder(mut self, folder: impl Into<String>) -> Self {
        self.movies_folder = folder.into();
        self
    }

    /// Route anime into its own subfolder (falls back to the TV folder).
    pub fn with_anime_folder(mut self, folder: Option<String>) -> Self {
        self.anime_folder = folder;
        self
    }

    /// The subfolder name for a category.
    pub fn category_folder(&self, category: Category) -> &str {
        self.category_folders
            .get(&category)
            .map(String::as_str)
            .unwrap_or("Other")
    }

    /// Compute the destination directory (without moving anything) for `src`.
    pub fn destination_for(&self, src: &Path, category: Category) -> PathBuf {
        self.destination_for_with_hints(src, category, &OrganizeHints::default())
    }

    /// Compute the destination directory, honoring caller routing hints.
    pub fn destination_for_with_hints(
        &self,
        src: &Path,
        category: Category,
        hints: &OrganizeHints,
    ) -> PathBuf {
        let media_kind = media_kind_for(src);

        // An explicit music subpath wins over artist/album parsing for every
        // audio file.
        if category == Category::Audio {
            if let Some(sub) = hints.music_path.as_deref().and_then(sanitize_rel_subpath) {
                return self
                    .library_root
                    .join(self.category_folder(category))
                    .join(sub);
            }
        }

        match (category, media_kind) {
            (Category::Video, media_kind) => {
                // Routing priority: explicit hint > anime heuristic > year
                // heuristic. An explicit `Tv` hint wins over a detected year
                // so "Movie.2019.S01E01"-style names still route to series.
                let anime = hints.media == Some(MediaHint::Anime)
                    || (hints.media.is_none() && looks_like_anime(src));
                let movie = hints.media == Some(MediaHint::Movie)
                    || (hints.media.is_none()
                        && !anime
                        && matches!(media_kind, MediaKind::Movie { .. }));
                let title = if anime {
                    anime_title_from_path(src).unwrap_or_else(|| "Unknown Title".to_string())
                } else {
                    title_from_path(src).unwrap_or_else(|| "Unknown Title".to_string())
                };
                if movie {
                    let year = match media_kind {
                        MediaKind::Movie { year } => year,
                        _ => None,
                    };
                    let base = self.library_root.join(&self.movies_folder);
                    match year {
                        Some(y) => base.join(format!("{title} ({y})")),
                        None => base.join(title),
                    }
                } else {
                    let season = match media_kind {
                        MediaKind::TvSeries { season } => season,
                        _ => None,
                    };
                    let year = match media_kind {
                        MediaKind::Movie { year } => year,
                        _ => None,
                    };
                    let base = if anime {
                        self.library_root
                            .join(self.anime_folder.as_deref().unwrap_or(&self.tv_folder))
                    } else {
                        self.library_root.join(&self.tv_folder)
                    };
                    match (season, year) {
                        (Some(n), _) => base.join(title).join(format!("Season {n:02}")),
                        // An anime film keeps the Plex `<Title> (<Year>)` form.
                        (None, Some(y)) => base.join(format!("{title} ({y})")),
                        (None, None) => base.join(title),
                    }
                }
            }
            (Category::Audio, MediaKind::Music { artist, album }) => {
                let artist = artist.unwrap_or_else(|| {
                    title_from_path(src).unwrap_or_else(|| "Unknown Artist".to_string())
                });
                self.library_root
                    .join(self.category_folder(category))
                    .join(artist)
                    .join(album.unwrap_or_else(|| "Unknown Album".to_string()))
            }
            // A lone audio track with no confident "Artist - Album" name:
            // land it directly in its album folder (mirroring its parent
            // directory) instead of deriving an extra filename subfolder.
            (Category::Audio, MediaKind::Other) => {
                let base = self
                    .library_root
                    .join(self.category_folder(Category::Audio));
                match src
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                {
                    Some(parent) => base.join(clean_title(parent)),
                    None => base,
                }
            }
            _ => self.library_root.join(self.category_folder(category)),
        }
    }

    /// Classify, compute the destination, and MOVE `src` into the library tree.
    /// Returns the new path on success.
    pub fn organize(&self, src: &Path) -> Result<PathBuf, Error> {
        let category = crate::classify::classify(src);
        self.organize_into(src, category)
    }

    /// Classify, honor routing hints, and MOVE `src` into the library tree.
    pub fn organize_with_hints(&self, src: &Path, hints: &OrganizeHints) -> Result<PathBuf, Error> {
        let category = crate::classify::classify(src);
        let destination_dir = self.destination_for_with_hints(src, category, hints);
        self.move_into(src, destination_dir)
    }

    /// Move `src` into the library tree under the given category.
    pub fn organize_into(&self, src: &Path, category: Category) -> Result<PathBuf, Error> {
        let destination_dir = self.destination_for(src, category);
        self.move_into(src, destination_dir)
    }

    /// Move `src` into the precomputed destination directory.
    fn move_into(&self, src: &Path, destination_dir: PathBuf) -> Result<PathBuf, Error> {
        if !src.is_file() {
            return Err(Error::Internal(format!(
                "organize: not a file: {}",
                src.display()
            )));
        }
        std::fs::create_dir_all(&destination_dir)
            .map_err(|e| Error::Internal(format!("organize: mkdir: {e}")))?;
        let file_name = src
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("download")
            .to_string();
        let destination = destination_dir.join(file_name);

        // Refuse to move onto itself or outside the library root. The
        // destination file does not exist yet, so containment is checked via
        // the (now existing) destination directory.
        if destination == src {
            return Ok(destination);
        }
        let root = std::fs::canonicalize(&self.library_root)
            .map_err(|e| Error::Internal(format!("organize: root: {e}")))?;
        let dir = std::fs::canonicalize(&destination_dir)
            .map_err(|e| Error::Internal(format!("organize: dest dir: {e}")))?;
        if !dir.starts_with(&root) {
            return Err(Error::UnsafePath);
        }
        if destination.exists() {
            return Err(Error::Internal(format!(
                "organize: destination already exists: {}",
                destination.display()
            )));
        }

        // Prefer an atomic same-volume rename; fall back to copy + delete when
        // the source and destination are on different drives.
        if let Err(e) = std::fs::rename(src, &destination) {
            tracing::warn!(
                source = %src.display(),
                destination = %destination.display(),
                error = %e,
                "organize: rename failed; falling back to copy"
            );
            std::fs::copy(src, &destination)
                .map_err(|ce| Error::Internal(format!("organize: copy: {ce}")))?;
            std::fs::remove_file(src)
                .map_err(|re| Error::Internal(format!("organize: remove source: {re}")))?;
        }
        Ok(destination)
    }
}

/// Determine how to nest `src` (uses the filename plus any folder hints such
/// as a `Season N` directory).
pub fn media_kind_for(src: &Path) -> MediaKind {
    let full = src
        .to_str()
        .map(|s| s.replace('\\', "/"))
        .unwrap_or_default();
    let file_name = src.file_name().and_then(|n| n.to_str()).unwrap_or("");

    // TV episode markers, most specific first.
    if let Some(season) = find_season(&full) {
        return MediaKind::TvSeries {
            season: Some(season),
        };
    }
    if let Some(season) = find_season(file_name) {
        return MediaKind::TvSeries {
            season: Some(season),
        };
    }
    // Folder hint like "Season 1".
    if let Some(season) = season_folder_hint(&full) {
        return MediaKind::TvSeries {
            season: Some(season),
        };
    }

    // Movie year markers.
    if let Some(year) = find_year(&full) {
        return MediaKind::Movie { year: Some(year) };
    }
    if let Some(year) = find_year(file_name) {
        return MediaKind::Movie { year: Some(year) };
    }

    // Music: "Artist - Album - NN - Track" filename pattern. Only nest a
    // real artist/album when the track name is explicit; otherwise fall
    // through to [`MediaKind::Other`] so a lone track is placed in its album
    // folder (parent directory) without an extra filename subfolder.
    if let Some((artist, album)) = parse_music(src) {
        return MediaKind::Music {
            artist: Some(artist),
            album: Some(album),
        };
    }

    MediaKind::Other
}

/// A cleaned, human-readable title derived from the path (dots and underscores
/// become spaces, common release tags are removed). For episodes the title is
/// truncated at the season marker; for movies at the year.
pub fn title_from_path(path: &Path) -> Option<String> {
    let candidate = path
        .file_stem()
        .and_then(|n| n.to_str())
        .or_else(|| path.to_str())?;
    let kind = media_kind_for(path);
    let cleaned = match kind {
        MediaKind::TvSeries { .. } => title_before_season(candidate),
        MediaKind::Movie { .. } => title_before_year(candidate),
        _ => clean_title(candidate),
    };
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

/// Fansub-aware title for anime releases:
/// `[Group] Title - 01 [1080p]` → `Title`, `[Judas] Show (Season 02)` → `Show`.
/// Strips leading bracketed group tags, cuts at `- NN` episode markers, and
/// drops a trailing `Season NN` phrase. Falls back to [`title_from_path`] for
/// anything it cannot confidently clean.
fn anime_title_from_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let mut rest = stem.trim();
    while rest.starts_with('[') {
        let close = rest.find(']')?;
        rest = rest[close + 1..]
            .trim_start()
            .trim_start_matches(['-', '_', ' '])
            .trim_start();
    }
    if rest.is_empty() {
        return None;
    }
    let mut title = rest.to_string();
    if let Some(idx) = index_after_dash_episode(&title) {
        title.truncate(idx);
        title = title.trim_end().to_string();
    }
    if let Some(idx) = index_of_sxxexx(&title) {
        title.truncate(idx);
        title = title.trim_end().to_string();
    }
    if let Some(idx) = index_of_season_phrase(&title) {
        title.truncate(idx);
        title = title.trim_end().to_string();
    }
    if let Some(idx) = index_of_year(&title) {
        title.truncate(idx);
        title = title.trim_end().to_string();
    }
    if title.trim().is_empty() {
        return None;
    }
    let cleaned = clean_title(&title);
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

/// Byte index of a standalone `(19|20)xx` year token, if any.
fn index_of_year(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 3 < bytes.len() {
        if bytes[i].is_ascii_digit() {
            if let Ok(year) = text[i..i + 4].parse::<u32>() {
                if (1900..=2100).contains(&year) {
                    return Some(i);
                }
            }
            i += 4;
        } else {
            i += 1;
        }
    }
    None
}

/// Byte index of a ` - NN ` episode marker (the dash), if any.
fn index_after_dash_episode(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] != b'-' || i + 1 >= bytes.len() {
            continue;
        }
        let mut j = i + 1;
        while j < bytes.len() && (bytes[j] as char).is_ascii_whitespace() {
            j += 1;
        }
        let digit_start = j;
        while j < bytes.len() && bytes[j].is_ascii_digit() && j - digit_start < 3 {
            j += 1;
        }
        if j > digit_start {
            return Some(i);
        }
    }
    None
}

/// Byte index of a standalone `SxxExx` season/episode marker, if any.
fn index_of_sxxexx(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 5 < bytes.len() {
        let starts_token = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        if starts_token
            && bytes[i].eq_ignore_ascii_case(&b'S')
            && bytes[i + 1].is_ascii_digit()
            && bytes[i + 2].is_ascii_digit()
            && bytes[i + 3].eq_ignore_ascii_case(&b'E')
            && bytes[i + 4].is_ascii_digit()
            && bytes[i + 5].is_ascii_digit()
        {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Byte index of a `Season NN` phrase (any bracket/space style).
fn index_of_season_phrase(text: &str) -> Option<usize> {
    let lower = text.to_lowercase();
    let bytes = lower.as_bytes();
    let mut from = 0;
    while let Some(rel) = lower[from..].find("season") {
        let idx = from + rel;
        let mut j = idx + 6;
        while j < bytes.len() && !bytes[j].is_ascii_digit() && (bytes[j] as char).is_whitespace() {
            j += 1;
        }
        if j < bytes.len() && bytes[j].is_ascii_digit() {
            return Some(idx);
        }
        from = idx + 6;
    }
    let _ = bytes;
    None
}

/// `S01E01` / `S1E05` / `S.01.E.02` markers (dots/spaces ignored).
fn find_season(text: &str) -> Option<u32> {
    let compact: String = text
        .to_uppercase()
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '.' && *c != '-' && *c != '_')
        .collect();
    let bytes = compact.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b != b'S' || i + 1 >= bytes.len() {
            continue;
        }
        let mut j = i + 1;
        let mut season: u32 = 0;
        let mut digits = 0;
        while j < bytes.len() && digits < 2 && bytes[j].is_ascii_digit() {
            season = season * 10 + (bytes[j] - b'0') as u32;
            j += 1;
            digits += 1;
        }
        if digits >= 1 && j < bytes.len() && bytes[j] == b'E' {
            return Some(season);
        }
    }
    None
}

/// The text before the `SxxEyy` episode marker, cleaned.
fn title_before_season(text: &str) -> String {
    let upper = text.to_uppercase();
    let compact: String = upper
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '.' && *c != '-' && *c != '_')
        .collect();
    let marker_len = compact.len();
    let mut cut = compact.len();
    for i in 0..marker_len {
        if compact.as_bytes()[i] == b'S' {
            let mut j = i + 1;
            let mut digits = 0;
            while j < compact.len() && digits < 2 && compact.as_bytes()[j].is_ascii_digit() {
                j += 1;
                digits += 1;
            }
            if digits >= 1 && j < compact.len() && compact.as_bytes()[j] == b'E' {
                cut = i;
                break;
            }
        }
    }
    // Rebuild the title from the original text using the compact index: map
    // the compact position back by walking the original. Simpler and safe:
    // strip the first occurrence of an episode marker via a regex-free scan.
    let lower = text.to_lowercase();
    let mut best = text.len();
    for i in 0..lower.len() {
        let b = lower.as_bytes()[i];
        if b == b's' && i + 1 < lower.len() {
            let mut j = i + 1;
            let mut digits = 0;
            while j < lower.len() && digits < 2 && lower.as_bytes()[j].is_ascii_digit() {
                j += 1;
                digits += 1;
            }
            if digits >= 1 && j < lower.len() && lower.as_bytes()[j] == b'e' {
                best = i;
                break;
            }
        }
    }
    let _ = cut;
    clean_title(&text[..best])
}

fn season_folder_hint(text: &str) -> Option<u32> {
    for part in text.split('/') {
        let lower = part.to_lowercase();
        let compact: String = lower.chars().filter(|c| !c.is_whitespace()).collect();
        if let Some(idx) = compact.find("season") {
            let after = &compact[idx + 6..];
            let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() {
                return digits.parse().ok();
            }
        }
    }
    None
}

fn find_year(text: &str) -> Option<u32> {
    let mut buffer = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            buffer.push(ch);
            if buffer.len() == 4 {
                if let Ok(year) = buffer.parse::<u32>() {
                    if (1900..=2100).contains(&year) {
                        return Some(year);
                    }
                }
                buffer.clear();
            }
        } else {
            buffer.clear();
        }
    }
    None
}

/// The text before the first `(19|20)xx` year, cleaned.
fn title_before_year(text: &str) -> String {
    let lower = text.to_lowercase();
    let bytes = lower.as_bytes();
    let mut best = text.len();
    let mut i = 0;
    while i + 3 < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let candidate = &text[i..i + 4];
            if let Ok(year) = candidate.parse::<u32>() {
                if (1900..=2100).contains(&year) {
                    best = i;
                    break;
                }
            }
        }
        i += 1;
    }
    clean_title(&text[..best])
}

/// Parse a "Artist - Album - 01 - Track" style filename into `(artist, album)`.
/// Returns `None` for anything ambiguous so lone tracks fall through to the
/// parent-directory album placement.
fn parse_music(path: &Path) -> Option<(String, String)> {
    let name = path.file_stem()?.to_str()?;
    let parts: Vec<&str> = name.split(" - ").map(str::trim).collect();
    if parts.len() >= 3 {
        return Some((clean_title(parts[0]), clean_title(parts[1])));
    }
    None
}

/// Conservative anime release detection: fansub-style releases name files
/// `[Group] Title - 01 [1080p].mkv`. Require BOTH a leading bracketed group
/// tag and an episode marker (`- NN`, `EP NN`, or `SxxEyy`) so ordinary scene
/// releases are never misrouted. Explicit hints always win over this.
pub fn looks_like_anime(src: &Path) -> bool {
    let Some(name) = src.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let trimmed = name.trim_start();
    if !trimmed.starts_with('[') {
        return false;
    }
    let Some(close) = trimmed.find(']') else {
        return false;
    };
    if close == 1 || close > 40 {
        return false;
    }
    if find_season(name).is_some() || season_folder_hint(&name.replace('\\', "/")).is_some() {
        return true;
    }
    // `- 01` / `- 01v2` episode numbering after the group tag.
    let rest = &trimmed[close + 1..];
    for part in rest.split(" - ").skip(1) {
        let digits: String = part
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if !digits.is_empty() {
            return true;
        }
    }
    false
}

/// Sanitize a caller-supplied relative subpath (`Artist/Album`). Rejects
/// traversal (`..`), drive/root components, and empty results. The final
/// containment check in [`Organizer::organize_into`] remains the hard guard.
fn sanitize_rel_subpath(raw: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for comp in raw.split(['/', '\\']) {
        let c = comp.trim();
        if c.is_empty() || c == "." {
            continue;
        }
        if c == ".." || c.contains(':') || c.starts_with('/') || c.starts_with('\\') {
            return None;
        }
        out.push(c);
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Release tags dropped when cleaning a title.
const TAGS: &[&str] = &[
    "1080P", "720P", "2160P", "4K", "X264", "X265", "H264", "H265", "BLURAY", "WEBRIP", "WEB-DL",
    "HDTV", "REMUX", "DVDRIP", "BRRIP", "AAC", "AC3", "DTS", "TRUEHD",
];

/// Normalize a title: separators become spaces, tags are removed, whitespace
/// is collapsed.
pub fn clean_title(raw: &str) -> String {
    let mut out = raw.replace(['.', '_', '-'], " ");
    for tag in TAGS {
        let mut removed = true;
        while removed {
            let upper = out.to_uppercase();
            match upper.find(tag) {
                Some(idx) => {
                    out.replace_range(idx..idx + tag.len(), " ");
                    removed = true;
                }
                None => removed = false,
            }
        }
    }
    let mut collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.retain(|c| !['(', ')', '[', ']'].contains(&c));
    collapsed.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tv_episode_parses() {
        let p = Path::new("Game.of.Thrones.S01E01.Winter.is.Coming.mkv");
        assert_eq!(media_kind_for(p), MediaKind::TvSeries { season: Some(1) });
        assert_eq!(title_from_path(p).unwrap(), "Game of Thrones");
    }

    #[test]
    fn season_folder_hint_parses() {
        let p = Path::new("The.Office/Season 2/The.Office.S02E05.mkv");
        assert_eq!(media_kind_for(p), MediaKind::TvSeries { season: Some(2) });
    }

    #[test]
    fn movie_year_parses() {
        let p = Path::new("The.Matrix.1999.1080p.BluRay.mkv");
        assert_eq!(media_kind_for(p), MediaKind::Movie { year: Some(1999) });
        assert_eq!(title_from_path(p).unwrap(), "The Matrix");
    }

    #[test]
    fn music_artist_album_parses() {
        let p = Path::new("Radiohead - OK Computer - 01 Airbag.flac");
        assert_eq!(
            media_kind_for(p),
            MediaKind::Music {
                artist: Some("Radiohead".to_string()),
                album: Some("OK Computer".to_string()),
            }
        );
    }

    #[test]
    fn destination_tree_is_jellyfin_friendly() {
        let organizer = Organizer::new(PathBuf::from("E:\\Media"));
        let show = Path::new("Game.of.Thrones.S01E01.Winter.is.Coming.mkv");
        let dst = organizer.destination_for(show, Category::Video);
        assert_eq!(
            dst,
            PathBuf::from("E:\\Media\\TV Shows\\Game of Thrones\\Season 01")
        );

        let movie = Path::new("The.Matrix.1999.1080p.mkv");
        let dst = organizer.destination_for(movie, Category::Video);
        assert_eq!(dst, PathBuf::from("E:\\Media\\Movies\\The Matrix (1999)"));

        let track = Path::new("Radiohead - OK Computer - 01 Airbag.flac");
        let dst = organizer.destination_for(track, Category::Audio);
        assert_eq!(
            dst,
            PathBuf::from("E:\\Media\\Music\\Radiohead\\OK Computer")
        );
    }

    #[test]
    fn organize_moves_file_into_tree() {
        let root = std::env::temp_dir().join(format!("agpeer-org-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let src = root.join("Show.Name.S01E01.mkv");
        std::fs::write(&src, b"episode").unwrap();

        let organizer = Organizer::new(root.clone());
        let new_path = organizer.organize(&src).unwrap();

        assert!(new_path.starts_with(root.join("TV Shows")));
        assert!(new_path
            .parent()
            .map(|p| p.ends_with("Season 01"))
            .unwrap_or(false));
        assert!(new_path.is_file());
        assert!(!src.exists());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn lone_track_lands_in_parent_album_folder() {
        let organizer = Organizer::new(PathBuf::from("E:\\Media"));
        // No "Artist - Album - NN - Track" pattern: a lone track should land in
        // a folder mirroring its parent directory, not a filename subfolder.
        let src = Path::new("E:/Media/Music/Soulseek/user/Some/Album/5-08 Rythm Of The Night.mp3");
        let dst = organizer.destination_for(src, Category::Audio);
        assert_eq!(dst, PathBuf::from("E:\\Media\\Music\\Album"));
    }

    #[test]
    fn folder_overrides_apply() {
        let organizer = Organizer::new(PathBuf::from("E:\\Media")).with_tv_folder("Shows");
        let show = Path::new("Game.of.Thrones.S01E01.mkv");
        assert_eq!(
            organizer.destination_for(show, Category::Video),
            PathBuf::from("E:\\Media\\Shows\\Game of Thrones\\Season 01")
        );
    }

    #[test]
    fn anime_heuristic_routes_to_anime_folder() {
        let organizer = Organizer::new(PathBuf::from("E:\\Media"))
            .with_tv_folder("Shows")
            .with_anime_folder(Some("Anime".to_string()));
        let ep = Path::new("[SubsPlease] Some Anime - 01 (1080p) [AAC].mkv");
        assert!(looks_like_anime(ep));
        assert_eq!(
            organizer.destination_for(ep, Category::Video),
            PathBuf::from("E:\\Media\\Anime\\Some Anime")
        );
    }

    #[test]
    fn scene_release_is_not_anime() {
        assert!(!looks_like_anime(Path::new(
            "Frieren.S02E10.1080p.CR.WEB-DL.mkv"
        )));
    }

    #[test]
    fn anime_hint_wins_over_year_and_folder_fallback() {
        let with_anime =
            Organizer::new(PathBuf::from("E:\\Media")).with_anime_folder(Some("Anime".to_string()));
        let hinted = Path::new("Anime.Movie.2019.1080p.BluRay.mkv");
        let hints = OrganizeHints {
            media: Some(MediaHint::Anime),
            ..Default::default()
        };
        assert_eq!(
            with_anime.destination_for_with_hints(hinted, Category::Video, &hints),
            PathBuf::from("E:\\Media\\Anime\\Anime Movie (2019)")
        );
        // Without a configured anime folder it falls back to the TV folder.
        let plain = Organizer::new(PathBuf::from("E:\\Media"));
        assert_eq!(
            plain.destination_for_with_hints(hinted, Category::Video, &hints),
            PathBuf::from("E:\\Media\\TV Shows\\Anime Movie (2019)")
        );
    }

    #[test]
    fn tv_hint_beats_movie_year_detection() {
        let organizer = Organizer::new(PathBuf::from("E:\\Media"));
        let odd = Path::new("Movie.The.Game.2019.S01E01.mkv");
        let hints = OrganizeHints {
            media: Some(MediaHint::Tv),
            ..Default::default()
        };
        assert_eq!(
            organizer.destination_for_with_hints(odd, Category::Video, &hints),
            PathBuf::from("E:\\Media\\TV Shows\\Movie The Game 2019\\Season 01")
        );
    }

    #[test]
    fn music_path_hint_routes_under_music_root() {
        let organizer = Organizer::new(PathBuf::from("E:\\Media"));
        let hints = OrganizeHints {
            music_path: Some("Daft Punk/Discovery".to_string()),
            ..Default::default()
        };
        assert_eq!(
            organizer.destination_for_with_hints(Path::new("track.mp3"), Category::Audio, &hints),
            PathBuf::from("E:\\Media\\Music\\Daft Punk\\Discovery")
        );
    }

    #[test]
    fn music_path_traversal_is_rejected_and_falls_back() {
        let organizer = Organizer::new(PathBuf::from("E:\\Media"));
        let bad = OrganizeHints {
            music_path: Some("../../Outside".to_string()),
            ..Default::default()
        };
        assert_eq!(
            organizer.destination_for_with_hints(
                Path::new("Artist - Album - 01 Song.flac"),
                Category::Audio,
                &bad
            ),
            PathBuf::from("E:\\Media\\Music\\Artist\\Album")
        );
        let drive = OrganizeHints {
            music_path: Some("C:/Windows/Temp".to_string()),
            ..Default::default()
        };
        assert_eq!(
            organizer.destination_for_with_hints(
                Path::new("Artist - Album - 01 Song.flac"),
                Category::Audio,
                &drive
            ),
            PathBuf::from("E:\\Media\\Music\\Artist\\Album")
        );
    }

    #[test]
    fn clean_title_removes_tags() {
        assert_eq!(
            clean_title("The.Matrix.1999.1080p.BluRay"),
            "The Matrix 1999"
        );
        assert_eq!(
            clean_title("Show.Name.S01E01.HDTV.x264"),
            "Show Name S01E01"
        );
    }
}
