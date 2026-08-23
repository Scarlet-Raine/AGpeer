//! Background housekeeping loops: search TTL expiry and transfer reconciliation.

use crate::state::AppState;
use agpeer_common::{Backend, Transfer, TransferId, TransferState};
use agpeer_storage::{SearchStore, TransferStore};
use chrono::Utc;
use serde_json::json;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Periodically expire stale searches, stop them on the backend, and purge
/// expired search results.
pub fn spawn_ttl_sweeper(state: Arc<AppState>, interval: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let store = SearchStore::new(&state.db);
            match store.expire_searches(Utc::now()).await {
                Ok(ids) => {
                    for id in ids {
                        state
                            .bus
                            .publish("search.expired", json!({ "search_id": id }));
                        if let Some(backend) = state.search_backend(Backend::Soulseek) {
                            if let Err(e) = backend.stop(&id).await {
                                tracing::warn!(search_id = %id, error = %e, "failed to stop expired search on backend");
                            }
                        }
                    }
                }
                Err(e) => tracing::warn!(error = %e, "failed to expire searches"),
            }
            if let Err(e) = store.purge_expired_results(Utc::now()).await {
                tracing::warn!(error = %e, "failed to purge expired search results");
            }
        }
    })
}

/// Periodically reconcile transfer state against each backend. The backend is
/// authoritative for existing transfers; records missing from the backend are
/// marked `Orphaned` (never deleted).
pub fn spawn_transfer_sync(
    state: Arc<AppState>,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let store = TransferStore::new(&state.db);
            for (backend, engine) in state.all_transfer_backends() {
                let list = match engine.list().await {
                    Ok(list) => list,
                    Err(e) => {
                        tracing::warn!(backend = %backend, error = %e, "transfer sync: backend list failed");
                        continue;
                    }
                };
                let ids: HashSet<TransferId> = list.iter().map(|t| t.id).collect();
                for t in &list {
                    let prev = store.get(&t.id).await.ok().flatten();
                    let is_new = prev.is_none();
                    let changed = prev
                        .as_ref()
                        .map(|p| {
                            p.state != t.state
                                || (p.progress - t.progress).abs() > 0.001
                                || p.download_rate != t.download_rate
                                || p.eta != t.eta
                        })
                        .unwrap_or(true);
                    if !(is_new || changed) {
                        continue;
                    }
                    if let Err(e) = store.upsert(t).await {
                        tracing::warn!(transfer_id = %t.id, error = %e, "transfer sync: upsert failed");
                    }
                    if let Err(e) = store.replace_files(&t.id, &t.files).await {
                        tracing::warn!(transfer_id = %t.id, error = %e, "transfer sync: replace files failed");
                    }
                    let kind = if is_new {
                        "transfer.added"
                    } else if t.state == TransferState::Completed {
                        "transfer.completed"
                    } else if t.state == TransferState::Failed {
                        "transfer.failed"
                    } else if matches!(prev.as_ref().map(|p| p.state), Some(TransferState::Queued))
                    {
                        "transfer.started"
                    } else {
                        "transfer.progress"
                    };
                    state.bus.publish(
                        kind,
                        json!({
                            "id": t.id,
                            "backend": backend.as_str(),
                            "state": t.state.as_str(),
                            "progress": t.progress,
                            "download_rate": t.download_rate,
                            "upload_rate": t.upload_rate,
                            "eta": t.eta,
                        }),
                    );

                    // Automatic post-processing: when a transfer first reaches
                    // Completed, move its files into the library tree.
                    if t.state == TransferState::Completed
                        && !prev
                            .as_ref()
                            .map(|p| p.state == TransferState::Completed)
                            .unwrap_or(false)
                        && crate::postprocess::auto_organize_enabled(&state)
                    {
                        match crate::postprocess::organize_completed_transfer(&state, t).await {
                            Ok(()) => {
                                tracing::info!(transfer_id = %t.id, "post-processed completed transfer");
                            }
                            Err(e) => {
                                tracing::warn!(transfer_id = %t.id, error = %e, "post-processing failed");
                            }
                        }
                    }
                }

                let db_list: Vec<Transfer> = store.list().await.ok().unwrap_or_default();
                for mut dt in db_list {
                    if dt.backend != backend || dt.state.is_terminal() || ids.contains(&dt.id) {
                        continue;
                    }
                    // A soulseek transfer missing from the backend but whose file
                    // is fully on disk means the download genuinely finished and
                    // the backend lost its in-memory record (restart/reconnect).
                    // Promote it to Completed instead of orphaning a completed
                    // download. Everything else still becomes Orphaned (files are
                    // never deleted).
                    if backend == Backend::Soulseek && soulseek_file_completed(&dt).unwrap_or(false)
                    {
                        dt.state = TransferState::Completed;
                        dt.progress = 1.0;
                        if dt.completed_at.is_none() {
                            dt.completed_at = Some(chrono::Utc::now());
                        }
                        if let Err(e) = store.upsert(&dt).await {
                            tracing::warn!(
                                transfer_id = %dt.id,
                                error = %e,
                                "transfer sync: completed-promote upsert failed"
                            );
                        }
                        state.bus.publish(
                            "transfer.completed",
                            json!({
                                "id": dt.id,
                                "backend": backend.as_str(),
                                "state": dt.state.as_str(),
                                "progress": 1.0,
                            }),
                        );
                        continue;
                    }
                    dt.state = TransferState::Orphaned;
                    dt.error = Some("missing from backend".into());
                    if let Err(e) = store.upsert(&dt).await {
                        tracing::warn!(transfer_id = %dt.id, error = %e, "transfer sync: orphan upsert failed");
                    }
                    state
                        .bus
                        .publish("transfer.orphaned", json!({ "id": dt.id }));
                }
            }
        }
    })
}

/// Whether a soulseek transfer's data file exists at its expected location
/// with the full expected size. Soulseek files land as
/// `<download_root>/<basename>`, so the check uses the filename's last path
/// segment against the transfer's destination. Returns `None` for non-soulseek
/// transfers or when the size/filename is unknown.
fn soulseek_file_completed(t: &Transfer) -> Option<bool> {
    if t.backend != Backend::Soulseek {
        return None;
    }
    let expected = t.bytes_total.filter(|b| *b > 0)?;
    let filename = t.metadata.get("soulseek")?.get("filename")?.as_str()?;
    let basename = filename.rsplit(['/', '\\']).next()?;
    let path = PathBuf::from(&t.destination).join(basename);
    Some(
        std::fs::metadata(&path)
            .map(|m| m.len() == expected)
            .unwrap_or(false),
    )
}
