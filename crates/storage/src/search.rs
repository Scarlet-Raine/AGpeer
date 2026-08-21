//! Search and search-result persistence.

use crate::{dt_to_text, text_to_dt, Database};
use agpeer_common::{Backend, ResultId, Search, SearchId, SearchResult, SearchState};
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use std::str::FromStr;

#[derive(Debug, Clone, FromRow)]
pub struct SearchRow {
    pub id: String,
    pub backend: String,
    pub query: String,
    pub state: String,
    pub result_count: i64,
    pub created_at: String,
    pub expires_at: String,
}

fn parse_search_state(s: &str) -> SearchState {
    match s {
        "pending" => SearchState::Pending,
        "active" => SearchState::Active,
        "completed" => SearchState::Completed,
        "failed" => SearchState::Failed,
        "expired" => SearchState::Expired,
        "stopped" => SearchState::Stopped,
        _ => SearchState::Pending,
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct SearchResultRow {
    pub result_id: String,
    pub search_id: String,
    pub username: String,
    pub path: String,
    pub filename: String,
    pub size: Option<i64>,
    pub extension: Option<String>,
    pub bitrate: Option<i64>,
    pub duration: Option<i64>,
    pub attributes: String,
    pub queue_length: Option<i64>,
    pub free_upload_slots: Option<i64>,
    pub upload_speed: Option<i64>,
    pub backend_metadata: String,
    /// TTL column; eviction is handled by SQL (`expires_at <= ?` in
    /// `purge_expired_results`), so the value is not read back.
    #[allow(dead_code)]
    pub expires_at: String,
}

impl SearchResultRow {
    pub fn into_result(self) -> Result<SearchResult, agpeer_common::Error> {
        Ok(SearchResult {
            result_id: ResultId::from_str(&self.result_id)?,
            search_id: SearchId::from_str(&self.search_id)?,
            username: self.username,
            path: self.path,
            filename: self.filename,
            size: self.size.map(|v| v.max(0) as u64),
            extension: self.extension,
            bitrate: self.bitrate.map(|v| v.max(0) as u32),
            duration: self.duration.map(|v| v.max(0) as u32),
            attributes: serde_json::from_str(&self.attributes).unwrap_or_default(),
            queue_length: self.queue_length.map(|v| v.max(0) as u32),
            free_upload_slots: self.free_upload_slots.map(|v| v != 0),
            upload_speed: self.upload_speed.map(|v| v.max(0) as u64),
            backend_metadata: serde_json::from_str(&self.backend_metadata).unwrap_or_default(),
        })
    }
}

impl SearchRow {
    pub fn into_search(self) -> Result<Search, agpeer_common::Error> {
        Ok(Search {
            id: SearchId::from_str(&self.id)?,
            backend: match self.backend.as_str() {
                "soulseek" => Backend::Soulseek,
                "hook" => Backend::Hook,
                _ => Backend::Torrent,
            },
            query: self.query,
            state: parse_search_state(&self.state),
            result_count: self.result_count.max(0) as usize,
            created_at: text_to_dt(&self.created_at)?,
            expires_at: text_to_dt(&self.expires_at)?,
        })
    }
}

pub struct SearchStore<'a> {
    db: &'a Database,
}

impl<'a> SearchStore<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub async fn upsert(&self, s: &Search) -> Result<(), agpeer_common::Error> {
        sqlx::query(
            r#"INSERT INTO searches (id, backend, query, state, result_count, created_at, expires_at)
               VALUES (?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                 state = excluded.state,
                 result_count = excluded.result_count"#,
        )
        .bind(s.id.to_string())
        .bind(s.backend.as_str())
        .bind(&s.query)
        .bind(s.state.as_str())
        .bind(s.result_count as i64)
        .bind(dt_to_text(s.created_at))
        .bind(dt_to_text(s.expires_at))
        .execute(self.db.pool())
        .await
        .map_err(|e| agpeer_common::Error::Database(e.to_string()))?;
        Ok(())
    }

    pub async fn get(&self, id: &SearchId) -> Result<Option<Search>, agpeer_common::Error> {
        let row: Option<SearchRow> = sqlx::query_as(
            "SELECT id, backend, query, state, result_count, created_at, expires_at FROM searches WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(self.db.pool())
        .await
        .map_err(|e| agpeer_common::Error::Database(e.to_string()))?;
        row.map(|r| r.into_search()).transpose()
    }

    pub async fn list(&self) -> Result<Vec<Search>, agpeer_common::Error> {
        let rows: Vec<SearchRow> = sqlx::query_as(
            "SELECT id, backend, query, state, result_count, created_at, expires_at FROM searches ORDER BY created_at DESC",
        )
        .fetch_all(self.db.pool())
        .await
        .map_err(|e| agpeer_common::Error::Database(e.to_string()))?;
        rows.into_iter().map(|r| r.into_search()).collect()
    }

    /// Insert a search result, upserting on its result id.
    pub async fn insert_result(
        &self,
        r: &SearchResult,
        expires_at: DateTime<Utc>,
    ) -> Result<(), agpeer_common::Error> {
        sqlx::query(
            r#"INSERT INTO search_results (
                 result_id, search_id, username, path, filename, size, extension,
                 bitrate, duration, attributes, queue_length, free_upload_slots,
                 upload_speed, backend_metadata, expires_at
               ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(result_id) DO UPDATE SET
                 search_id = excluded.search_id,
                 username = excluded.username,
                 path = excluded.path,
                 filename = excluded.filename,
                 size = excluded.size,
                 extension = excluded.extension,
                 bitrate = excluded.bitrate,
                 duration = excluded.duration,
                 attributes = excluded.attributes,
                 queue_length = excluded.queue_length,
                 free_upload_slots = excluded.free_upload_slots,
                 upload_speed = excluded.upload_speed,
                 backend_metadata = excluded.backend_metadata,
                 expires_at = excluded.expires_at"#,
        )
        .bind(r.result_id.to_string())
        .bind(r.search_id.to_string())
        .bind(&r.username)
        .bind(&r.path)
        .bind(&r.filename)
        .bind(r.size.map(|v| v as i64))
        .bind(&r.extension)
        .bind(r.bitrate.map(|v| v as i64))
        .bind(r.duration.map(|v| v as i64))
        .bind(serde_json::to_string(&r.attributes).unwrap_or_else(|_| "{}".into()))
        .bind(r.queue_length.map(|v| v as i64))
        .bind(r.free_upload_slots.map(|v| v as i64))
        .bind(r.upload_speed.map(|v| v as i64))
        .bind(serde_json::to_string(&r.backend_metadata).unwrap_or_else(|_| "{}".into()))
        .bind(dt_to_text(expires_at))
        .execute(self.db.pool())
        .await
        .map_err(|e| agpeer_common::Error::Database(e.to_string()))?;
        Ok(())
    }

    pub async fn results(&self, id: &SearchId) -> Result<Vec<SearchResult>, agpeer_common::Error> {
        let rows: Vec<SearchResultRow> = sqlx::query_as(
            r#"SELECT result_id, search_id, username, path, filename, size, extension,
                 bitrate, duration, attributes, queue_length, free_upload_slots,
                 upload_speed, backend_metadata, expires_at
                 FROM search_results WHERE search_id = ? ORDER BY rowid"#,
        )
        .bind(id.to_string())
        .fetch_all(self.db.pool())
        .await
        .map_err(|e| agpeer_common::Error::Database(e.to_string()))?;
        rows.into_iter().map(|r| r.into_result()).collect()
    }

    pub async fn get_result(
        &self,
        id: &ResultId,
    ) -> Result<Option<SearchResult>, agpeer_common::Error> {
        let row: Option<SearchResultRow> = sqlx::query_as(
            r#"SELECT result_id, search_id, username, path, filename, size, extension,
                 bitrate, duration, attributes, queue_length, free_upload_slots,
                 upload_speed, backend_metadata, expires_at
                 FROM search_results WHERE result_id = ?"#,
        )
        .bind(id.to_string())
        .fetch_optional(self.db.pool())
        .await
        .map_err(|e| agpeer_common::Error::Database(e.to_string()))?;
        row.map(|r| r.into_result()).transpose()
    }

    /// Delete results whose TTL has expired, and return the search ids that
    /// had results removed.
    pub async fn purge_expired_results(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<SearchId>, agpeer_common::Error> {
        let affected: Vec<String> = sqlx::query_scalar(
            "DELETE FROM search_results WHERE expires_at <= ? RETURNING search_id",
        )
        .bind(dt_to_text(now))
        .fetch_all(self.db.pool())
        .await
        .map_err(|e| agpeer_common::Error::Database(e.to_string()))?;
        affected
            .into_iter()
            .map(|s| {
                SearchId::from_str(&s).map_err(|e| agpeer_common::Error::Database(e.to_string()))
            })
            .collect()
    }

    /// Expire searches past their TTL.
    pub async fn expire_searches(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<SearchId>, agpeer_common::Error> {
        let rows: Vec<String> = sqlx::query_scalar(
            "UPDATE searches SET state = 'expired' WHERE expires_at <= ? AND state NOT IN ('expired','stopped') RETURNING id",
        )
        .bind(dt_to_text(now))
        .fetch_all(self.db.pool())
        .await
        .map_err(|e| agpeer_common::Error::Database(e.to_string()))?;
        rows.into_iter()
            .map(|s| {
                SearchId::from_str(&s).map_err(|e| agpeer_common::Error::Database(e.to_string()))
            })
            .collect()
    }
}
