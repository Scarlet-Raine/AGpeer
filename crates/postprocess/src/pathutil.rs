pub fn canonicalize_safe(
    path: &std::path::Path,
) -> Result<std::path::PathBuf, agpeer_common::Error> {
    std::fs::canonicalize(path).map_err(|e| agpeer_common::Error::Internal(e.to_string()))
}

pub fn is_within(root: &std::path::Path, path: &std::path::Path) -> bool {
    let root = match std::fs::canonicalize(root) {
        Ok(r) => r,
        Err(_) => return false,
    };
    let path = match std::fs::canonicalize(path) {
        Ok(p) => p,
        Err(_) => return false,
    };
    path.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalize_safe_works_for_temp_file() {
        let dir = std::env::temp_dir();
        let file = dir.join("pathutil_test.txt");
        std::fs::write(&file, "test").unwrap();
        let canonical = std::fs::canonicalize(&file).unwrap();
        let result = canonicalize_safe(&file);
        std::fs::remove_file(&file).unwrap();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), canonical);
    }

    #[test]
    fn is_within_true_for_child() {
        let root = std::env::temp_dir();
        let child = root.join("child");
        std::fs::create_dir_all(&child).unwrap();
        assert!(is_within(&root, &child));
        std::fs::remove_dir(&child).unwrap();
    }

    #[test]
    fn is_within_false_for_outside() {
        let root = std::env::temp_dir().join("pathutil_root");
        std::fs::create_dir_all(&root).unwrap();
        let outside = std::env::temp_dir().join("pathutil_outside");
        std::fs::create_dir_all(&outside).unwrap();
        assert!(!is_within(&root, &outside));
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir_all(&outside).unwrap();
    }
}
