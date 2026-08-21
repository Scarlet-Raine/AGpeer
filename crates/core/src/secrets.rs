//! Secret storage backends.
//!
//! Secrets (Soulseek credentials, API keys) are stored via an abstraction so
//! the rest of the application never cares whether the value lives in the OS
//! keyring, a file, or memory. Values must never be logged.

use agpeer_common::Error;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// A storage abstraction for opaque secret values keyed by name.
pub trait SecretStore: Send + Sync {
    /// Retrieve a secret, or `None` if it does not exist.
    fn get(&self, key: &str) -> Result<Option<String>, Error>;

    /// Store a secret, overwriting any existing value.
    fn set(&self, key: &str, value: &str) -> Result<(), Error>;

    /// Remove a secret. Removing a missing key is not an error.
    fn delete(&self, key: &str) -> Result<(), Error>;
}

/// Secret store backed by the OS keyring via the `keyring` crate.
pub struct KeyringSecretStore {
    service: String,
}

impl KeyringSecretStore {
    pub fn new() -> Self {
        Self {
            service: "agpeer".into(),
        }
    }
}

impl SecretStore for KeyringSecretStore {
    fn get(&self, key: &str) -> Result<Option<String>, Error> {
        let entry =
            keyring::Entry::new(&self.service, key).map_err(|e| Error::Internal(e.to_string()))?;
        match entry.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(Error::Internal(e.to_string())),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<(), Error> {
        let entry =
            keyring::Entry::new(&self.service, key).map_err(|e| Error::Internal(e.to_string()))?;
        entry
            .set_password(value)
            .map_err(|e| Error::Internal(e.to_string()))
    }

    fn delete(&self, key: &str) -> Result<(), Error> {
        let entry =
            keyring::Entry::new(&self.service, key).map_err(|e| Error::Internal(e.to_string()))?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(Error::Internal(e.to_string())),
        }
    }
}

impl Default for KeyringSecretStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Secret store backed by one file per key inside a directory.
///
/// Intended for environments without an OS keyring; the directory must have
/// restrictive permissions (application-owned private data directory).
pub struct FileSecretStore {
    dir: PathBuf,
}

impl FileSecretStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}

impl SecretStore for FileSecretStore {
    fn get(&self, key: &str) -> Result<Option<String>, Error> {
        let path = self.dir.join(key);
        match std::fs::read_to_string(&path) {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::Internal(e.to_string())),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<(), Error> {
        std::fs::create_dir_all(&self.dir).map_err(|e| Error::Internal(e.to_string()))?;
        let path = self.dir.join(key);
        std::fs::write(&path, value).map_err(|e| Error::Internal(e.to_string()))
    }

    fn delete(&self, key: &str) -> Result<(), Error> {
        let path = self.dir.join(key);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::Internal(e.to_string())),
        }
    }
}

/// In-memory secret store, useful for tests and ephemeral processes.
pub struct MemorySecretStore {
    inner: Mutex<HashMap<String, String>>,
}

impl MemorySecretStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for MemorySecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self, key: &str) -> Result<Option<String>, Error> {
        let inner = self
            .inner
            .lock()
            .map_err(|e| Error::Internal(e.to_string()))?;
        Ok(inner.get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> Result<(), Error> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|e| Error::Internal(e.to_string()))?;
        inner.insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), Error> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|e| Error::Internal(e.to_string()))?;
        inner.remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_roundtrip_and_delete() {
        let store = MemorySecretStore::new();
        assert_eq!(store.get("token").unwrap(), None);
        store.set("token", "s3cr3t").unwrap();
        assert_eq!(store.get("token").unwrap(), Some("s3cr3t".to_string()));
        store.delete("token").unwrap();
        assert_eq!(store.get("token").unwrap(), None);
        store.delete("token").unwrap();
    }

    #[test]
    fn file_roundtrip_in_temp_dir() {
        let dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let store = FileSecretStore::new(dir.clone());
        assert_eq!(store.get("key").unwrap(), None);
        store.set("key", "value").unwrap();
        assert_eq!(store.get("key").unwrap(), Some("value".to_string()));
        store.delete("key").unwrap();
        assert_eq!(store.get("key").unwrap(), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
