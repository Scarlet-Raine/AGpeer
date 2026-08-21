use std::path::{Path, PathBuf};

pub trait Extractor: Send + Sync {
    fn supports(&self, archive: &Path) -> bool;
    fn extract(&self, archive: &Path, dest: &Path) -> Result<(), agpeer_common::Error>;
}

pub struct SevenZipExtractor {
    pub binary: PathBuf,
}

impl Default for SevenZipExtractor {
    fn default() -> Self {
        Self {
            binary: PathBuf::from("7z"),
        }
    }
}

impl Extractor for SevenZipExtractor {
    fn supports(&self, archive: &Path) -> bool {
        archive
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .map(|ext| {
                matches!(
                    ext.as_str(),
                    "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "xz"
                )
            })
            .unwrap_or(false)
    }

    fn extract(&self, archive: &Path, dest: &Path) -> Result<(), agpeer_common::Error> {
        std::fs::create_dir_all(dest)
            .map_err(|io| agpeer_common::Error::Internal(io.to_string()))?;
        let output = std::process::Command::new(&self.binary)
            .arg("x")
            .arg("-y")
            .arg(archive)
            .arg(format!("-o{}", dest.display()))
            .output()
            .map_err(|io| agpeer_common::Error::Internal(io.to_string()))?;
        if !output.status.success() {
            return Err(agpeer_common::Error::ExtractionFailed);
        }
        Ok(())
    }
}

pub fn sanitize_entry_path(entry: &str, dest: &Path) -> Result<PathBuf, agpeer_common::Error> {
    if entry.is_empty() || entry.contains('\0') {
        return Err(agpeer_common::Error::UnsafePath);
    }
    let normalized = entry.replace('\\', "/");
    if normalized.starts_with('/') {
        return Err(agpeer_common::Error::UnsafePath);
    }
    let bytes = normalized.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return Err(agpeer_common::Error::UnsafePath);
    }
    if normalized.split('/').any(|comp| comp == "..") {
        return Err(agpeer_common::Error::UnsafePath);
    }
    Ok(dest.join(normalized))
}

pub fn is_multipart(archive: &Path) -> bool {
    let name = archive
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if name.is_empty() {
        return false;
    }
    let ext = name.rsplit('.').next().unwrap_or("");
    if ext.len() == 3
        && ext.starts_with('r')
        && ext.as_bytes()[1].is_ascii_digit()
        && ext.as_bytes()[2].is_ascii_digit()
    {
        return true;
    }
    if name.contains(".part") && name.ends_with(".rar") {
        return true;
    }
    // 7z split-volume sets: "x.7z.001" (last segment is a 3-digit part number).
    if ext.len() == 3 && ext.as_bytes().iter().all(|b| b.is_ascii_digit()) && name.contains(".7z.")
    {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_entry_path_ok() {
        let dest = Path::new("/tmp/out");
        assert_eq!(
            sanitize_entry_path("folder/file.txt", dest).unwrap(),
            dest.join("folder/file.txt")
        );
    }

    #[test]
    fn sanitize_entry_path_rejects_unsafe() {
        let dest = Path::new("/tmp/out");
        for bad in [
            "../../evil.exe",
            "..\\evil",
            "/etc/passwd",
            "C:\\Windows",
            "\\\\srv\\share",
            "a/\0b",
        ] {
            assert_eq!(
                sanitize_entry_path(bad, dest).unwrap_err().code(),
                "UnsafePath",
                "entry: {bad}"
            );
        }
    }

    #[test]
    fn is_multipart_true() {
        for name in ["x.r00", "x.part2.rar", "x.7z.001"] {
            assert!(is_multipart(Path::new(name)), "name: {name}");
        }
    }

    #[test]
    fn is_multipart_false() {
        for name in ["x.rar", "x.zip", "x.r"] {
            assert!(!is_multipart(Path::new(name)), "name: {name}");
        }
    }
}
