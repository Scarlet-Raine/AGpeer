//! SQLite persistence layer.
//!
//! SQLite is the source of truth for all application-owned state. Backend
//! state is reconciled against the database on startup.

mod backend_state;
mod event;
mod jobs;
mod search;
mod settings;
mod transfer;

pub use backend_state::BackendStateStore;
pub use event::{Event, EventStore};
pub use jobs::{JobRow, JobStore};
pub use search::{SearchRow, SearchStore};
pub use settings::SettingsStore;
pub use transfer::{TransferFileRow, TransferRow, TransferStore};

use agpeer_common::{Error, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;

/// Handle to the application database.
#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    /// Open (creating if necessary) a SQLite database at `path`.
    pub async fn open(path: &str) -> Result<Self> {
        let options = SqliteConnectOptions::from_str(path)
            .map_err(|e| Error::Database(e.to_string()))?
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(options)
            .await
            .map_err(|e| Error::Database(e.to_string()))?;
        Ok(Self { pool })
    }

    /// Wrap an existing pool.
    ///
    /// Callers connecting to an in-memory database must limit the pool to a
    /// single connection; multiple connections to `sqlite::memory:` each get
    /// their own private database.
    pub fn from_pool(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Run pending migrations.
    pub async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .map_err(|e| Error::Database(e.to_string()))?;
        Ok(())
    }

    /// The underlying pool.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

/// Helper to serialize a datetime as RFC3339 text for storage.
pub(crate) fn dt_to_text(dt: chrono::DateTime<chrono::Utc>) -> String {
    dt.to_rfc3339()
}

/// Helper to parse RFC3339 text back into a datetime.
pub(crate) fn text_to_dt(s: &str) -> Result<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&chrono::Utc))
        .map_err(Error::InvalidTimestamp)
}

/// In-memory database helper shared by tests across modules.
///
/// The pool is limited to one connection because SQLite in-memory databases
/// are per-connection; a multi-connection pool would silently create several
/// independent databases and break tests.
#[cfg(test)]
pub(crate) async fn mem_db() -> Database {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect in-memory db");
    let db = Database::from_pool(pool);
    db.migrate().await.expect("migrate");
    db
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn migrations_run_on_empty_db() {
        let db = mem_db().await;
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'")
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert!(count >= 9);
    }

    #[tokio::test]
    async fn migrations_run_on_file_db() {
        let dir = std::env::temp_dir().join(format!("agpeer-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test.db");
        {
            let db = Database::open(db_path.to_str().unwrap()).await.unwrap();
            db.migrate().await.unwrap();
            let count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'")
                    .fetch_one(db.pool())
                    .await
                    .unwrap();
            assert!(count >= 9);
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
