use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Audio,
    Video,
    Image,
    Archive,
    Document,
    Software,
    Unknown,
}

impl std::fmt::Display for Category {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Category::Audio => "audio",
            Category::Video => "video",
            Category::Image => "image",
            Category::Archive => "archive",
            Category::Document => "document",
            Category::Software => "software",
            Category::Unknown => "unknown",
        };
        write!(f, "{name}")
    }
}

pub fn classify(path: &Path) -> Category {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());
    match ext.as_deref() {
        Some("zip") | Some("rar") | Some("7z") | Some("tar") | Some("gz") | Some("tgz")
        | Some("bz2") | Some("xz") => Category::Archive,
        Some("mp3") | Some("flac") | Some("wav") | Some("ogg") | Some("m4a") | Some("aac")
        | Some("opus") | Some("wma") => Category::Audio,
        Some("mp4") | Some("mkv") | Some("avi") | Some("mov") | Some("webm") | Some("m4v")
        | Some("ts") => Category::Video,
        Some("jpg") | Some("jpeg") | Some("png") | Some("gif") | Some("webp") | Some("bmp")
        | Some("tiff") | Some("svg") | Some("heic") => Category::Image,
        Some("pdf") | Some("epub") | Some("mobi") | Some("doc") | Some("docx") | Some("xls")
        | Some("xlsx") | Some("ppt") | Some("pptx") | Some("txt") | Some("md") | Some("rtf") => {
            Category::Document
        }
        Some("exe") | Some("msi") | Some("apk") | Some("dmg") | Some("deb") | Some("rpm")
        | Some("appimage") => Category::Software,
        _ => Category::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn categories_map_correctly() {
        assert_eq!(classify(Path::new("file.zip")), Category::Archive);
        assert_eq!(classify(Path::new("file.7z")), Category::Archive);
        assert_eq!(classify(Path::new("file.tar.gz")), Category::Archive);
        assert_eq!(classify(Path::new("file.mp3")), Category::Audio);
        assert_eq!(classify(Path::new("song.FLAC")), Category::Audio);
        assert_eq!(classify(Path::new("movie.mp4")), Category::Video);
        assert_eq!(classify(Path::new("movie.mkv")), Category::Video);
        assert_eq!(classify(Path::new("photo.jpg")), Category::Image);
        assert_eq!(classify(Path::new("photo.PNG")), Category::Image);
        assert_eq!(classify(Path::new("book.pdf")), Category::Document);
        assert_eq!(classify(Path::new("notes.md")), Category::Document);
        assert_eq!(classify(Path::new("setup.exe")), Category::Software);
        assert_eq!(classify(Path::new("app.AppImage")), Category::Software);
    }

    #[test]
    fn unknown_extension_is_unknown() {
        assert_eq!(classify(Path::new("file.xyz")), Category::Unknown);
        assert_eq!(classify(Path::new("no_extension")), Category::Unknown);
    }
}
