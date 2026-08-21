//! Post-processing engine: classification, safe extraction, media organization,
//! and the observable/retryable step pipeline.

pub mod classify;
pub mod extract;
pub mod organize;
pub mod pathutil;
pub mod pipeline;

pub use classify::{classify, Category};
pub use extract::{is_multipart, sanitize_entry_path, Extractor, SevenZipExtractor};
pub use organize::{clean_title, media_kind_for, title_from_path, MediaKind, Organizer};
pub use pathutil::{canonicalize_safe, is_within};
pub use pipeline::{InstallerPolicy, Pipeline};
