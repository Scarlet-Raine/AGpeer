//! Backend state persistence (used for reconciliation across restarts).

use crate::{dt_to_text, text_to_dt, Database};
use agpeer_common::{Backend, Error};
use chrono::{DateTime, Utc};

/// State of a backend as last observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendState {
    Ready,
    Degraded,
    Stopped,
}

impl BackendState {
    pub fn as_str(&self) -> &'static str {
        match self {
            BackendState::Ready => "ready",
            BackendState::Degraded => "degraded",
            BackendState::Stopped => "stopped",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "degraded" => BackendState::Degraded,
            "stopped" => BackendState::Stopped,
            _ => BackendState::Ready,
        }
    }
}

pub struct BackendStateStore<'a> {
    db: &'a Database,
}

impl<'a> BackendStateStore<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub async fn set(&self, backend: Backend, state: BackendState) -> Result<(), Error> {
        sqlx::query(
            "INSERT INTO backend_state (backend, state, updated_at) VALUES (?, ?, ?)
             ON CONFLICT(backend) DO UPDATE SET state = excluded.state, updated_at = excluded.updated_at",
        )
        .bind(backend.as_str())
        .bind(state.as_str())
        .bind(dt_to_text(Utc::now()))
        .execute(self.db.pool())
        .await
        .map_err(|e| Error::Database(e.to_string()))?;
        Ok(())
    }

    pub async fn get(
        &self,
        backend: Backend,
    ) -> Result<Option<(BackendState, DateTime<Utc>)>, Error> {
        let row: Option<(String, String)> =
            sqlx::query_as("SELECT state, updated_at FROM backend_state WHERE backend = ?")
                .bind(backend.as_str())
                .fetch_optional(self.db.pool())
                .await
                .map_err(|e| Error::Database(e.to_string()))?;
        row.map(|(s, t)| Ok((BackendState::parse(&s), text_to_dt(&t)?)))
            .transpose()
    }
}
