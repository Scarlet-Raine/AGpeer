//! Automatic post-processing on transfer completion.
//!
//! When a transfer finishes, its completed files are organized into the
//! configured library root (`E:\Media` by default) under a Jellyfin/Plex
//! friendly tree: category → title → season. The run is recorded as a
//! [`agpeer_jobs::Job`] (observable via `GET /api/v1/postprocess`) and each
//! step publishes `postprocess.*` events.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use agpeer_common::{Error, Transfer, TransferId};
use agpeer_jobs::{Job, JobState, Step, StepKind, StepState};
use agpeer_postprocess::Organizer;
use agpeer_storage::JobStore;
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use crate::state::AppState;

/// Whether automatic organization is enabled for the running configuration.
pub fn auto_organize_enabled(state: &AppState) -> bool {
    state.config.postprocess.auto_organize
        && !state.config.postprocess.library_root.trim().is_empty()
}

/// Build an organizer for the configured library root.
fn organizer(state: &AppState) -> Organizer {
    Organizer::new(PathBuf::from(&state.config.postprocess.library_root))
}

/// Resolve the absolute path of a completed file within a transfer.
fn resolve_file_path(transfer: &Transfer, relative_path: &str) -> PathBuf {
    PathBuf::from(&transfer.destination).join(relative_path)
}

/// Remove now-empty directories from `start`'s parent upwards, stopping at
/// (and not including) `root`. Used to tidy download staging folders after a
/// completed file has been moved into the library.
fn prune_empty_dirs(root: &Path, start: &Path) {
    let Ok(root) = std::fs::canonicalize(root) else {
        return;
    };
    let mut dir = start.parent().and_then(|p| std::fs::canonicalize(p).ok());
    while let Some(d) = dir {
        if d == root || !d.starts_with(&root) {
            break;
        }
        let empty = std::fs::read_dir(&d)
            .map(|mut it| it.next().is_none())
            .unwrap_or(false);
        if !empty {
            break;
        }
        if std::fs::remove_dir(&d).is_err() {
            break;
        }
        dir = d.parent().and_then(|p| std::fs::canonicalize(p).ok());
    }
}

/// Organize the completed files of `transfer` into the library tree.
///
/// Each file is a `move` step in a recorded job. The job is persisted through
/// `GET /api/v1/postprocess`; the transfer's `postprocess_state` is updated
/// through the returned state (the caller persists the transfer).
pub async fn organize_completed_transfer(
    state: &Arc<AppState>,
    transfer: &Transfer,
) -> Result<(), Error> {
    let organizer = organizer(state);

    let steps = vec![
        Step {
            index: 0,
            kind: StepKind::Verify,
            state: StepState::Pending,
            started_at: None,
            completed_at: None,
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
    ];
    let mut job = Job {
        id: Uuid::new_v4(),
        transfer_id: transfer.id,
        target: transfer.display_name.clone(),
        state: JobState::Running,
        steps,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        error: None,
    };

    let store = JobStore::new(&state.db);
    store.upsert(&job).await?;
    state.bus.publish(
        "postprocess.started",
        json!({"id": job.id, "transfer_id": transfer.id}),
    );

    // Files to organize: the transfer's file list (torrents) or the display
    // name fallback (soulseek single files carry one entry after reconcile).
    let files: Vec<(String, PathBuf)> = if transfer.files.is_empty() {
        vec![(
            transfer.display_name.clone(),
            PathBuf::from(&transfer.destination).join(&transfer.display_name),
        )]
    } else {
        transfer
            .files
            .iter()
            .map(|f| (f.path.clone(), resolve_file_path(transfer, &f.path)))
            .collect()
    };

    for (step_idx, (relative, absolute)) in files.iter().enumerate() {
        let step = job.steps.get_mut(1).unwrap();
        step.state = StepState::Running;
        step.started_at = Some(Utc::now());
        job.updated_at = Utc::now();
        state.bus.publish(
            "postprocess.step_started",
            json!({"job_id": job.id, "step": "move", "file": relative}),
        );

        let result = if !absolute.is_file() {
            Err(Error::Internal(format!(
                "completed file missing: {}",
                absolute.display()
            )))
        } else {
            organizer.organize(absolute).map(|_| ())
        };

        match result {
            Ok(()) => {
                step.state = StepState::Completed;
                step.completed_at = Some(Utc::now());
                // Tidy the staging dir: remove now-empty ancestor folders left
                // behind by the move, stopping at the transfer's destination.
                prune_empty_dirs(Path::new(&transfer.destination), absolute);
                state.bus.publish(
                    "postprocess.step_completed",
                    json!({"job_id": job.id, "step": "move", "file": relative}),
                );
            }
            Err(e) => {
                step.state = StepState::Failed;
                step.error = Some(e.to_string());
                job.state = JobState::Failed;
                job.error = Some(format!("{}: {e}", relative));
                job.updated_at = Utc::now();
                store.upsert(&job).await?;
                state.bus.publish(
                    "postprocess.failed",
                    json!({"id": job.id, "transfer_id": transfer.id, "error": e.to_string()}),
                );
                return Err(e);
            }
        }

        let _ = step_idx;
    }

    job.steps[0].state = StepState::Completed;
    job.steps[0].completed_at = Some(Utc::now());
    job.state = JobState::Completed;
    job.updated_at = Utc::now();
    store.upsert(&job).await?;
    state.bus.publish(
        "postprocess.completed",
        json!({"id": job.id, "transfer_id": transfer.id}),
    );
    Ok(())
}

/// Convenience: mark a transfer's post-process state on the in-memory copy.
pub fn mark_state(transfer: &mut Transfer, state: agpeer_common::PostprocessState) {
    transfer.postprocess_state = state;
}

/// Transfer id helper re-exported for housekeeping callers.
pub fn transfer_key(id: &TransferId) -> String {
    id.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use agpeer_common::{Backend, PostprocessState, TransferFile, TransferState};
    use agpeer_storage::Database;
    use chrono::{DateTime, Utc};

    async fn mem_db() -> Database {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect in-memory db");
        let db = Database::from_pool(pool);
        db.migrate().await.expect("migrate");
        db
    }

    fn completed_transfer(destination: String) -> Transfer {
        Transfer {
            id: TransferId::new(),
            backend: Backend::Torrent,
            source: "magnet:?xt=urn:btih:test".into(),
            display_name: "Show.Name.S01E01.mkv".into(),
            state: TransferState::Completed,
            progress: 1.0,
            bytes_total: Some(1024),
            bytes_completed: 1024,
            download_rate: None,
            upload_rate: None,
            eta: None,
            destination,
            created_at: Utc::now(),
            started_at: Some(Utc::now()),
            completed_at: Some(Utc::now()),
            error: None,
            files: vec![TransferFile {
                index: "0".into(),
                path: "Show.Name.S01E01.mkv".into(),
                size: 1024,
                selected: true,
                bytes_completed: 1024,
            }],
            postprocess_state: PostprocessState::None,
            metadata: Default::default(),
        }
    }

    #[tokio::test]
    async fn organize_completed_transfer_moves_file_and_records_job() {
        let base = std::env::temp_dir().join(format!("agpeer-pp-{}", uuid::Uuid::new_v4()));
        let downloads = base.join("downloads");
        let library = base.join("library");
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::write(downloads.join("Show.Name.S01E01.mkv"), b"episode").unwrap();

        let config = crate::config::AppConfig {
            postprocess: crate::config::PostprocessConfig {
                library_root: library.to_string_lossy().into_owned(),
                auto_organize: true,
            },
            ..crate::config::AppConfig::default()
        };
        let state = AppState::new(config, mem_db().await, "token".into());
        let transfer = completed_transfer(downloads.to_string_lossy().into_owned());

        organize_completed_transfer(&state, &transfer)
            .await
            .expect("organize should succeed");

        // The file moved into the Jellyfin tree.
        let organized = library
            .join("TV Shows")
            .join("Show Name")
            .join("Season 01")
            .join("Show.Name.S01E01.mkv");
        assert!(
            organized.is_file(),
            "file missing at {}",
            organized.display()
        );
        assert!(!downloads.join("Show.Name.S01E01.mkv").exists());

        // A completed job was recorded and is queryable via the postprocess API.
        let store = JobStore::new(&state.db);
        let jobs = store.list_for_transfer(&transfer.id).await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].state, JobState::Completed);

        let _: DateTime<Utc> = Utc::now();
        std::fs::remove_dir_all(&base).unwrap();
    }
}
