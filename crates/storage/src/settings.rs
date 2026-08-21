//! Runtime-settable settings persistence (SQLite `settings` table).

use crate::Database;
use agpeer_common::Error;
use serde_json::Value;

pub struct SettingsStore<'a> {
    db: &'a Database,
}

impl<'a> SettingsStore<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Read a single setting as raw JSON.
    pub async fn get(&self, key: &str) -> Result<Option<Value>, Error> {
        let raw: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(self.db.pool())
            .await
            .map_err(|e| Error::Database(e.to_string()))?;
        raw.map(|s| serde_json::from_str(&s).map_err(|e| Error::Database(e.to_string())))
            .transpose()
    }

    /// Read a single setting as a typed value.
    pub async fn get_typed<T: serde::de::DeserializeOwned>(
        &self,
        key: &str,
    ) -> Result<Option<T>, Error> {
        let raw: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(self.db.pool())
            .await
            .map_err(|e| Error::Database(e.to_string()))?;
        raw.map(|s| serde_json::from_str(&s).map_err(|e| Error::Database(e.to_string())))
            .transpose()
    }

    /// Set a single setting (JSON-encoded).
    pub async fn set(&self, key: &str, value: &Value) -> Result<(), Error> {
        let encoded = serde_json::to_string(value).map_err(|e| Error::Database(e.to_string()))?;
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(encoded)
        .execute(self.db.pool())
        .await
        .map_err(|e| Error::Database(e.to_string()))?;
        Ok(())
    }

    /// Remove a setting.
    pub async fn delete(&self, key: &str) -> Result<(), Error> {
        sqlx::query("DELETE FROM settings WHERE key = ?")
            .bind(key)
            .execute(self.db.pool())
            .await
            .map_err(|e| Error::Database(e.to_string()))?;
        Ok(())
    }

    /// Read all settings into a JSON object.
    pub async fn all(&self) -> Result<serde_json::Map<String, Value>, Error> {
        let rows: Vec<(String, String)> = sqlx::query_as("SELECT key, value FROM settings")
            .fetch_all(self.db.pool())
            .await
            .map_err(|e| Error::Database(e.to_string()))?;
        let mut map = serde_json::Map::new();
        for (k, v) in rows {
            if let Ok(value) = serde_json::from_str(&v) {
                map.insert(k, value);
            }
        }
        Ok(map)
    }
}
