//! Post-processing job and step model.
//!
//! A transfer can spawn zero or more post-processing jobs; each job targets a
//! specific file within the transfer and runs an ordered, individually
//! observable and retryable list of steps.

use agpeer_common::TransferId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The ordered pipeline steps supported in v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Verify,
    Extract,
    Flatten,
    Rename,
    InspectMedia,
    Move,
    Copy,
    Hardlink,
    Cleanup,
    RunInstaller,
    CustomHook,
}

impl StepKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            StepKind::Verify => "verify",
            StepKind::Extract => "extract",
            StepKind::Flatten => "flatten",
            StepKind::Rename => "rename",
            StepKind::InspectMedia => "inspect_media",
            StepKind::Move => "move",
            StepKind::Copy => "copy",
            StepKind::Hardlink => "hardlink",
            StepKind::Cleanup => "cleanup",
            StepKind::RunInstaller => "run_installer",
            StepKind::CustomHook => "custom_hook",
        }
    }
}

/// State of a single step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepState {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

impl StepState {
    pub fn as_str(&self) -> &'static str {
        match self {
            StepState::Pending => "pending",
            StepState::Running => "running",
            StepState::Completed => "completed",
            StepState::Failed => "failed",
            StepState::Skipped => "skipped",
        }
    }
}

/// State of a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobState {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobState::Pending => "pending",
            JobState::Running => "running",
            JobState::Completed => "completed",
            JobState::Failed => "failed",
            JobState::Cancelled => "cancelled",
        }
    }
}

/// A single step within a post-processing job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub index: u32,
    pub kind: StepKind,
    pub state: StepState,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

/// A post-processing job bound to a transfer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: uuid::Uuid,
    pub transfer_id: TransferId,
    /// The file (transfer file index/path) this job targets.
    pub target: String,
    pub state: JobState,
    pub steps: Vec<Step>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_kind_names_match_pipeline() {
        assert_eq!(StepKind::Verify.as_str(), "verify");
        assert_eq!(StepKind::Extract.as_str(), "extract");
        assert_eq!(StepKind::InspectMedia.as_str(), "inspect_media");
        assert_eq!(StepKind::RunInstaller.as_str(), "run_installer");
        assert_eq!(StepKind::CustomHook.as_str(), "custom_hook");
    }

    #[test]
    fn step_kind_serde_roundtrip() {
        let cases = [
            StepKind::Verify,
            StepKind::Extract,
            StepKind::Flatten,
            StepKind::Rename,
            StepKind::InspectMedia,
            StepKind::Move,
            StepKind::Copy,
            StepKind::Hardlink,
            StepKind::Cleanup,
            StepKind::RunInstaller,
            StepKind::CustomHook,
        ];
        for k in cases {
            let json = serde_json::to_string(&k).unwrap();
            let back: StepKind = serde_json::from_str(&json).unwrap();
            assert_eq!(k, back);
        }
    }

    #[test]
    fn state_serde_roundtrip() {
        for s in [
            StepState::Pending,
            StepState::Running,
            StepState::Completed,
            StepState::Failed,
            StepState::Skipped,
        ] {
            let json = serde_json::to_string(&s).unwrap();
            let back: StepState = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back);
        }
        for s in [
            JobState::Pending,
            JobState::Running,
            JobState::Completed,
            JobState::Failed,
            JobState::Cancelled,
        ] {
            let json = serde_json::to_string(&s).unwrap();
            let back: JobState = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back);
        }
    }

    #[test]
    fn job_serializes_with_transfer_id() {
        let job = Job {
            id: uuid::Uuid::new_v4(),
            transfer_id: TransferId::new(),
            target: "disk1/file.rar".into(),
            state: JobState::Pending,
            steps: vec![Step {
                index: 0,
                kind: StepKind::Extract,
                state: StepState::Pending,
                started_at: None,
                completed_at: None,
                error: None,
            }],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            error: None,
        };
        let json = serde_json::to_string(&job).unwrap();
        let back: Job = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, job.id);
        assert_eq!(back.transfer_id, job.transfer_id);
        assert_eq!(back.target, job.target);
        assert_eq!(back.steps[0].kind, StepKind::Extract);
    }

    #[test]
    fn step_state_names_match_contract() {
        assert_eq!(StepState::Pending.as_str(), "pending");
        assert_eq!(StepState::Running.as_str(), "running");
        assert_eq!(StepState::Skipped.as_str(), "skipped");
        assert_eq!(JobState::Cancelled.as_str(), "cancelled");
    }
}
