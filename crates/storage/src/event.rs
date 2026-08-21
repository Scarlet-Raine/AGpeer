//! Audit/event log persistence.

use crate::{dt_to_text, Database};
use agpeer_common::Error;
use chrono::{DateTime, Utc};
use sqlx::FromRow;

/// A persisted audit/event entry.
#[derive(Debug, Clone, FromRow)]
pub struct Event {
    pub id: i64,
    pub ts: String,
    pub kind: String,
    pub payload: String,
}

pub struct EventStore<'a> {
    db: &'a Database,
}

impl<'a> EventStore<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Append an audit event.
    pub async fn append(&self, kind: &str, payload: &serde_json::Value) -> Result<(), Error> {
        let payload = serde_json::to_string(payload).unwrap_or_else(|_| "{}".into());
        sqlx::query("INSERT INTO events (ts, kind, payload) VALUES (?, ?, ?)")
            .bind(dt_to_text(Utc::now()))
            .bind(kind)
            .bind(payload)
            .execute(self.db.pool())
            .await
            .map_err(|e| Error::Database(e.to_string()))?;
        Ok(())
    }

    /// Read recent events (most recent first), up to `limit`.
    pub async fn recent(&self, limit: i64) -> Result<Vec<Event>, Error> {
        let rows: Vec<Event> =
            sqlx::query_as("SELECT id, ts, kind, payload FROM events ORDER BY id DESC LIMIT ?")
                .bind(limit)
                .fetch_all(self.db.pool())
                .await
                .map_err(|e| Error::Database(e.to_string()))?;
        Ok(rows)
    }
}

/// Convenience for converting a stored event into its deserialized payload.
impl Event {
    pub fn parse_payload<T: serde::de::DeserializeOwned>(&self) -> Option<T> {
        serde_json::from_str(&self.payload).ok()
    }

    pub fn timestamp(&self) -> Option<DateTime<Utc>> {
        chrono::DateTime::parse_from_rfc3339(&self.ts)
            .map(|d| d.with_timezone(&Utc))
            .ok()
    }
}
