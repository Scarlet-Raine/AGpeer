//! Transfer persistence.

use crate::{dt_to_text, text_to_dt, Database};
use agpeer_common::{Backend, PostprocessState, Transfer, TransferFile, TransferId, TransferState};
use serde::Deserialize;
use sqlx::FromRow;
use std::str::FromStr;

/// Raw database row for a transfer (ids/datetimes kept as text/raw SQL types).
#[derive(Debug, Clone, FromRow)]
pub struct TransferRow {
    pub id: String,
    pub backend: String,
    pub source: String,
    pub display_name: String,
    pub state: String,
    pub progress: f32,
    pub bytes_total: Option<i64>,
    pub bytes_completed: i64,
    pub download_rate: Option<i64>,
    pub upload_rate: Option<i64>,
    pub eta: Option<i64>,
    pub destination: String,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub error: Option<String>,
    pub postprocess_state: String,
    pub metadata: String,
}

/// Raw database row for a transfer file.
#[derive(Debug, Clone, FromRow, Deserialize)]
pub struct TransferFileRow {
    pub transfer_id: String,
    pub file_index: String,
    pub path: String,
    pub size: i64,
    pub selected: i64,
    pub bytes_completed: i64,
}

fn parse_state(s: &str) -> TransferState {
    match s {
        "queued" => TransferState::Queued,
        "resolving" => TransferState::Resolving,
        "downloading" => TransferState::Downloading,
        "paused" => TransferState::Paused,
        "verifying" => TransferState::Verifying,
        "completed" => TransferState::Completed,
        "postprocessing" => TransferState::Postprocessing,
        "ready" => TransferState::Ready,
        "failed" => TransferState::Failed,
        "cancelled" => TransferState::Cancelled,
        "orphaned" => TransferState::Orphaned,
        _ => TransferState::Queued,
    }
}

fn parse_backend(s: &str) -> Backend {
    match s {
        "soulseek" => Backend::Soulseek,
        "hook" => Backend::Hook,
        _ => Backend::Torrent,
    }
}

fn parse_postprocess(s: &str) -> PostprocessState {
    match s {
        "pending" => PostprocessState::Pending,
        "running" => PostprocessState::Running,
        "completed" => PostprocessState::Completed,
        "failed" => PostprocessState::Failed,
        _ => PostprocessState::None,
    }
}

impl TransferRow {
    /// Convert a database row into the normalized model.
    pub fn into_transfer(self, files: Vec<TransferFile>) -> Result<Transfer, agpeer_common::Error> {
        let metadata = serde_json::from_str(&self.metadata).unwrap_or_default();
        Ok(Transfer {
            id: TransferId::from_str(&self.id)?,
            backend: parse_backend(&self.backend),
            source: self.source,
            display_name: self.display_name,
            state: parse_state(&self.state),
            progress: self.progress.clamp(0.0, 1.0),
            bytes_total: self.bytes_total.map(|v| v.max(0) as u64),
            bytes_completed: self.bytes_completed.max(0) as u64,
            download_rate: self.download_rate.map(|v| v.max(0) as u64),
            upload_rate: self.upload_rate.map(|v| v.max(0) as u64),
            eta: self.eta.map(|v| v.max(0) as u64),
            destination: self.destination,
            created_at: text_to_dt(&self.created_at)?,
            started_at: self.started_at.as_deref().map(text_to_dt).transpose()?,
            completed_at: self.completed_at.as_deref().map(text_to_dt).transpose()?,
            error: self.error,
            files,
            postprocess_state: parse_postprocess(&self.postprocess_state),
            metadata,
        })
    }
}

impl From<&TransferFile> for TransferFileRow {
    fn from(f: &TransferFile) -> Self {
        Self {
            transfer_id: String::new(),
            file_index: f.index.clone(),
            path: f.path.clone(),
            size: f.size as i64,
            selected: f.selected as i64,
            bytes_completed: f.bytes_completed as i64,
        }
    }
}

/// Persistence operations for transfers and their files.
pub struct TransferStore<'a> {
    db: &'a Database,
}

impl<'a> TransferStore<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Insert or replace a transfer row (without files).
    pub async fn upsert(&self, t: &Transfer) -> Result<(), agpeer_common::Error> {
        let metadata = serde_json::to_string(&t.metadata).unwrap_or_else(|_| "{}".to_string());
        sqlx::query(
            r#"INSERT INTO transfers (
                id, backend, source, display_name, state, progress,
                bytes_total, bytes_completed, download_rate, upload_rate, eta,
                destination, created_at, started_at, completed_at, error,
                postprocess_state, metadata
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                backend = excluded.backend,
                source = excluded.source,
                display_name = excluded.display_name,
                state = excluded.state,
                progress = excluded.progress,
                bytes_total = excluded.bytes_total,
                bytes_completed = excluded.bytes_completed,
                download_rate = excluded.download_rate,
                upload_rate = excluded.upload_rate,
                eta = excluded.eta,
                destination = excluded.destination,
                started_at = excluded.started_at,
                completed_at = excluded.completed_at,
                error = excluded.error,
                postprocess_state = excluded.postprocess_state,
                metadata = excluded.metadata"#,
        )
        .bind(t.id.to_string())
        .bind(t.backend.as_str())
        .bind(&t.source)
        .bind(&t.display_name)
        .bind(t.state.as_str())
        .bind(t.progress)
        .bind(t.bytes_total.map(|v| v as i64))
        .bind(t.bytes_completed as i64)
        .bind(t.download_rate.map(|v| v as i64))
        .bind(t.upload_rate.map(|v| v as i64))
        .bind(t.eta.map(|v| v as i64))
        .bind(&t.destination)
        .bind(dt_to_text(t.created_at))
        .bind(t.started_at.map(dt_to_text))
        .bind(t.completed_at.map(dt_to_text))
        .bind(&t.error)
        .bind(t.postprocess_state.as_str())
        .bind(metadata)
        .execute(self.db.pool())
        .await
        .map_err(|e| agpeer_common::Error::Database(e.to_string()))?;
        Ok(())
    }

    /// Replace the files for a transfer.
    pub async fn replace_files(
        &self,
        transfer_id: &TransferId,
        files: &[TransferFile],
    ) -> Result<(), agpeer_common::Error> {
        let mut tx = self
            .db
            .pool()
            .begin()
            .await
            .map_err(|e| agpeer_common::Error::Database(e.to_string()))?;
        sqlx::query("DELETE FROM transfer_files WHERE transfer_id = ?")
            .bind(transfer_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(|e| agpeer_common::Error::Database(e.to_string()))?;
        for f in files {
            sqlx::query(
                "INSERT INTO transfer_files (transfer_id, file_index, path, size, selected, bytes_completed) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(transfer_id.to_string())
            .bind(&f.index)
            .bind(&f.path)
            .bind(f.size as i64)
            .bind(f.selected as i64)
            .bind(f.bytes_completed as i64)
            .execute(&mut *tx)
            .await
            .map_err(|e| agpeer_common::Error::Database(e.to_string()))?;
        }
        tx.commit()
            .await
            .map_err(|e| agpeer_common::Error::Database(e.to_string()))?;
        Ok(())
    }

    /// Fetch files for a transfer.
    pub async fn files(
        &self,
        transfer_id: &TransferId,
    ) -> Result<Vec<TransferFile>, agpeer_common::Error> {
        let rows: Vec<TransferFileRow> =
            sqlx::query_as("SELECT transfer_id, file_index, path, size, selected, bytes_completed FROM transfer_files WHERE transfer_id = ? ORDER BY file_index")
                .bind(transfer_id.to_string())
                .fetch_all(self.db.pool())
                .await
                .map_err(|e| agpeer_common::Error::Database(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(|r| TransferFile {
                index: r.file_index,
                path: r.path,
                size: r.size.max(0) as u64,
                selected: r.selected != 0,
                bytes_completed: r.bytes_completed.max(0) as u64,
            })
            .collect())
    }

    /// Fetch a single transfer plus its files.
    pub async fn get(&self, id: &TransferId) -> Result<Option<Transfer>, agpeer_common::Error> {
        let row: Option<TransferRow> = sqlx::query_as(
            r#"SELECT id, backend, source, display_name, state, progress,
                bytes_total, bytes_completed, download_rate, upload_rate, eta,
                destination, created_at, started_at, completed_at, error,
                postprocess_state, metadata
                FROM transfers WHERE id = ?"#,
        )
        .bind(id.to_string())
        .fetch_optional(self.db.pool())
        .await
        .map_err(|e| agpeer_common::Error::Database(e.to_string()))?;
        match row {
            None => Ok(None),
            Some(r) => {
                let files = self.files(id).await?;
                Ok(Some(r.into_transfer(files)?))
            }
        }
    }

    /// List all transfers plus their files.
    pub async fn list(&self) -> Result<Vec<Transfer>, agpeer_common::Error> {
        let rows: Vec<TransferRow> = sqlx::query_as(
            r#"SELECT id, backend, source, display_name, state, progress,
                bytes_total, bytes_completed, download_rate, upload_rate, eta,
                destination, created_at, started_at, completed_at, error,
                postprocess_state, metadata
                FROM transfers ORDER BY created_at DESC"#,
        )
        .fetch_all(self.db.pool())
        .await
        .map_err(|e| agpeer_common::Error::Database(e.to_string()))?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let id = TransferId::from_str(&r.id)?;
            let files = self.files(&id).await?;
            out.push(r.into_transfer(files)?);
        }
        Ok(out)
    }

    /// Delete a transfer and its files.
    pub async fn delete(&self, id: &TransferId) -> Result<(), agpeer_common::Error> {
        let mut tx = self
            .db
            .pool()
            .begin()
            .await
            .map_err(|e| agpeer_common::Error::Database(e.to_string()))?;
        sqlx::query("DELETE FROM transfer_files WHERE transfer_id = ?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(|e| agpeer_common::Error::Database(e.to_string()))?;
        sqlx::query("DELETE FROM transfers WHERE id = ?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(|e| agpeer_common::Error::Database(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| agpeer_common::Error::Database(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample() -> Transfer {
        Transfer {
            id: TransferId::new(),
            backend: Backend::Torrent,
            source: "magnet:?xt=urn:btih:test".into(),
            display_name: "test.torrent".into(),
            state: TransferState::Downloading,
            progress: 0.5,
            bytes_total: Some(1000),
            bytes_completed: 500,
            download_rate: Some(10),
            upload_rate: None,
            eta: Some(50),
            destination: "C:\\tmp".into(),
            created_at: Utc::now(),
            started_at: Some(Utc::now()),
            completed_at: None,
            error: None,
            files: vec![TransferFile {
                index: "0".into(),
                path: "test.torrent".into(),
                size: 1000,
                selected: true,
                bytes_completed: 500,
            }],
            postprocess_state: PostprocessState::None,
            metadata: Default::default(),
        }
    }

    #[tokio::test]
    async fn upsert_get_roundtrip() {
        let db = crate::mem_db().await;
        let store = TransferStore::new(&db);
        let t = sample();
        store.upsert(&t).await.unwrap();
        store.replace_files(&t.id, &t.files).await.unwrap();
        let got = store.get(&t.id).await.unwrap().unwrap();
        assert_eq!(got.id, t.id);
        assert_eq!(got.state, TransferState::Downloading);
        assert_eq!(got.files.len(), 1);
    }

    #[tokio::test]
    async fn delete_removes_transfer_and_files() {
        let db = crate::mem_db().await;
        let store = TransferStore::new(&db);
        let t = sample();
        store.upsert(&t).await.unwrap();
        store.replace_files(&t.id, &t.files).await.unwrap();
        store.delete(&t.id).await.unwrap();
        assert!(store.get(&t.id).await.unwrap().is_none());
        assert!(store.files(&t.id).await.unwrap().is_empty());
    }
}
