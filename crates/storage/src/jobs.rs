//! Post-process job persistence.
//!
//! Jobs are stored in the `postprocess_jobs` table. The ordered step list of a
//! job is persisted as a JSON document in the `steps` column; the
//! `postprocess_steps` table is reserved for a later wave that stores steps
//! individually.

use crate::{dt_to_text, text_to_dt, Database};
use agpeer_common::{Error, Result, TransferId};
use agpeer_jobs::{Job, JobState, Step};
use sqlx::FromRow;
use std::str::FromStr;
use uuid::Uuid;

/// Raw database row for a post-process job.
#[derive(Debug, Clone, FromRow)]
pub struct JobRow {
    pub id: String,
    pub transfer_id: String,
    pub target: String,
    pub state: String,
    pub steps: String,
    pub created_at: String,
    pub updated_at: String,
    pub error: Option<String>,
}

fn parse_job_state(s: &str) -> JobState {
    match s {
        "running" => JobState::Running,
        "completed" => JobState::Completed,
        "failed" => JobState::Failed,
        "cancelled" => JobState::Cancelled,
        _ => JobState::Pending,
    }
}

impl JobRow {
    /// Convert a database row into the normalized job model.
    pub fn into_job(self) -> Result<Job> {
        let steps: Vec<Step> =
            serde_json::from_str(&self.steps).map_err(|e| Error::Database(e.to_string()))?;
        Ok(Job {
            id: Uuid::from_str(&self.id)?,
            transfer_id: TransferId::from_str(&self.transfer_id)?,
            target: self.target,
            state: parse_job_state(&self.state),
            steps,
            created_at: text_to_dt(&self.created_at)?,
            updated_at: text_to_dt(&self.updated_at)?,
            error: self.error,
        })
    }
}

/// Persistence operations for post-process jobs.
pub struct JobStore<'a> {
    db: &'a Database,
}

impl<'a> JobStore<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Insert or update a job (keyed on its id). `created_at` is preserved on
    /// update; `updated_at` is replaced.
    pub async fn upsert(&self, job: &Job) -> Result<()> {
        let steps =
            serde_json::to_string(&job.steps).map_err(|e| Error::Database(e.to_string()))?;
        sqlx::query(
            r#"INSERT INTO postprocess_jobs (id, transfer_id, target, state, steps, created_at, updated_at, error)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                 transfer_id = excluded.transfer_id,
                 target = excluded.target,
                 state = excluded.state,
                 steps = excluded.steps,
                 updated_at = excluded.updated_at,
                 error = excluded.error"#,
        )
        .bind(job.id.to_string())
        .bind(job.transfer_id.to_string())
        .bind(&job.target)
        .bind(job.state.as_str())
        .bind(steps)
        .bind(dt_to_text(job.created_at))
        .bind(dt_to_text(job.updated_at))
        .bind(&job.error)
        .execute(self.db.pool())
        .await
        .map_err(|e| Error::Database(e.to_string()))?;
        Ok(())
    }

    /// Fetch a single job by id.
    pub async fn get(&self, id: &Uuid) -> Result<Option<Job>> {
        let row: Option<JobRow> = sqlx::query_as(
            "SELECT id, transfer_id, target, state, steps, created_at, updated_at, error FROM postprocess_jobs WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(self.db.pool())
        .await
        .map_err(|e| Error::Database(e.to_string()))?;
        row.map(|r| r.into_job()).transpose()
    }

    /// List all jobs, most recently updated first.
    pub async fn list(&self) -> Result<Vec<Job>> {
        let rows: Vec<JobRow> = sqlx::query_as(
            "SELECT id, transfer_id, target, state, steps, created_at, updated_at, error FROM postprocess_jobs ORDER BY created_at DESC",
        )
        .fetch_all(self.db.pool())
        .await
        .map_err(|e| Error::Database(e.to_string()))?;
        rows.into_iter().map(|r| r.into_job()).collect()
    }

    /// List all jobs for a transfer, most recently created first.
    pub async fn list_for_transfer(&self, transfer_id: &TransferId) -> Result<Vec<Job>> {
        let rows: Vec<JobRow> = sqlx::query_as(
            "SELECT id, transfer_id, target, state, steps, created_at, updated_at, error FROM postprocess_jobs WHERE transfer_id = ? ORDER BY created_at DESC",
        )
        .bind(transfer_id.to_string())
        .fetch_all(self.db.pool())
        .await
        .map_err(|e| Error::Database(e.to_string()))?;
        rows.into_iter().map(|r| r.into_job()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agpeer_jobs::{Step, StepKind, StepState};
    use chrono::Utc;

    fn sample_job(transfer_id: TransferId, target: &str) -> Job {
        Job {
            id: Uuid::new_v4(),
            transfer_id,
            target: target.to_string(),
            state: JobState::Running,
            steps: vec![
                Step {
                    index: 0,
                    kind: StepKind::Extract,
                    state: StepState::Completed,
                    started_at: Some(Utc::now()),
                    completed_at: Some(Utc::now()),
                    error: None,
                },
                Step {
                    index: 1,
                    kind: StepKind::Move,
                    state: StepState::Pending,
                    started_at: None,
                    completed_at: None,
                    error: None,
                },
            ],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            error: None,
        }
    }

    #[tokio::test]
    async fn upsert_get_roundtrip_preserves_steps() {
        let db = crate::mem_db().await;
        let store = JobStore::new(&db);
        let job = sample_job(TransferId::new(), "disk1/file.rar");
        store.upsert(&job).await.unwrap();

        let got = store.get(&job.id).await.unwrap().unwrap();
        assert_eq!(got.id, job.id);
        assert_eq!(got.transfer_id, job.transfer_id);
        assert_eq!(got.target, job.target);
        assert_eq!(got.state, JobState::Running);
        assert_eq!(got.steps.len(), 2);
        assert_eq!(got.steps[0].kind, StepKind::Extract);
        assert_eq!(got.steps[0].state, StepState::Completed);
        assert_eq!(got.steps[1].state, StepState::Pending);
        assert_eq!(got.created_at, job.created_at);
    }

    #[tokio::test]
    async fn upsert_updates_in_place() {
        let db = crate::mem_db().await;
        let store = JobStore::new(&db);
        let transfer_id = TransferId::new();
        let mut job = sample_job(transfer_id, "disk1/file.rar");
        store.upsert(&job).await.unwrap();

        job.state = JobState::Completed;
        job.error = Some("boom".into());
        store.upsert(&job).await.unwrap();

        let got = store.get(&job.id).await.unwrap().unwrap();
        assert_eq!(got.state, JobState::Completed);
        assert_eq!(got.error.as_deref(), Some("boom"));
        assert_eq!(store.list().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn list_filters_by_transfer() {
        let db = crate::mem_db().await;
        let store = JobStore::new(&db);
        let t1 = TransferId::new();
        let t2 = TransferId::new();

        store.upsert(&sample_job(t1, "a.rar")).await.unwrap();
        store.upsert(&sample_job(t1, "b.rar")).await.unwrap();
        store.upsert(&sample_job(t2, "c.rar")).await.unwrap();

        let for_t1 = store.list_for_transfer(&t1).await.unwrap();
        assert_eq!(for_t1.len(), 2);
        assert_eq!(store.list().await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn get_missing_returns_none() {
        let db = crate::mem_db().await;
        let store = JobStore::new(&db);
        assert!(store.get(&Uuid::new_v4()).await.unwrap().is_none());
    }
}
