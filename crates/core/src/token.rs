//! API token management.

use agpeer_common::Error;
use std::path::Path;

pub fn ensure_token(data_dir: &Path) -> Result<String, Error> {
    let path = data_dir.join("token");
    if let Ok(contents) = std::fs::read_to_string(&path) {
        let trimmed = contents.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    let token = uuid::Uuid::new_v4().to_string();
    std::fs::create_dir_all(data_dir).map_err(|e| Error::Internal(e.to_string()))?;
    std::fs::write(&path, &token).map_err(|e| Error::Internal(e.to_string()))?;
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_token_and_is_stable() {
        let dir = std::env::temp_dir().join(format!("agpeer-token-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let first = ensure_token(&dir).unwrap();
        assert!(!first.is_empty());
        assert!(dir.join("token").exists());

        let second = ensure_token(&dir).unwrap();
        assert_eq!(first, second);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
