use agpeer_common::Error;
use agpeer_jobs::{Job, JobState, StepKind, StepState};
use chrono::Utc;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InstallerPolicy {
    #[default]
    Deny,
    Ask,
    Allow,
}

pub struct Pipeline {
    pub extractor: Arc<dyn crate::extract::Extractor>,
    pub installer_policy: InstallerPolicy,
}

impl Pipeline {
    pub fn new(extractor: Arc<dyn crate::extract::Extractor>) -> Self {
        Self {
            extractor,
            installer_policy: InstallerPolicy::Deny,
        }
    }
}

pub fn resume_from(job: &Job) -> Option<usize> {
    job.steps
        .iter()
        .position(|step| step.state != StepState::Completed)
}

impl Pipeline {
    pub async fn run(
        &self,
        job: &mut Job,
        work_dir: &std::path::Path,
        dest_dir: &std::path::Path,
    ) -> Result<(), Error> {
        let start = resume_from(job).unwrap_or(job.steps.len());
        let step_count = job.steps.len();
        for i in start..step_count {
            job.steps[i].state = StepState::Running;
            job.steps[i].started_at = Some(Utc::now());
            job.updated_at = Utc::now();

            let outcome: Result<(), Error> = (async {
                let kind = job.steps[i].kind;
                match kind {
                    StepKind::Verify => {
                        let p = work_dir.join(&job.target);
                        if !p.exists()
                            || tokio::fs::metadata(&p).await.map(|m| m.len()).unwrap_or(0) == 0
                        {
                            return Err(Error::ExtractionFailed);
                        }
                    }
                    StepKind::Extract => {
                        let src = work_dir.join(&job.target);
                        if self.extractor.supports(&src) {
                            self.extractor.extract(&src, dest_dir)?;
                        } else {
                            return Err(Error::ExtractionFailed);
                        }
                    }
                    StepKind::Flatten => {
                        let mut entries = tokio::fs::read_dir(work_dir)
                            .await
                            .map_err(|e| Error::Internal(e.to_string()))?;
                        while let Some(entry) = entries
                            .next_entry()
                            .await
                            .map_err(|e| Error::Internal(e.to_string()))?
                        {
                            if !entry
                                .file_type()
                                .await
                                .map_err(|e| Error::Internal(e.to_string()))?
                                .is_dir()
                            {
                                continue;
                            }
                            let dir_path = entry.path();
                            let mut children = tokio::fs::read_dir(&dir_path)
                                .await
                                .map_err(|e| Error::Internal(e.to_string()))?;
                            while let Some(child) = children
                                .next_entry()
                                .await
                                .map_err(|e| Error::Internal(e.to_string()))?
                            {
                                let dest = work_dir.join(child.file_name());
                                tokio::fs::rename(child.path(), &dest)
                                    .await
                                    .map_err(|e| Error::Internal(e.to_string()))?;
                            }
                            tokio::fs::remove_dir(&dir_path)
                                .await
                                .map_err(|e| Error::Internal(e.to_string()))?;
                        }
                    }
                    StepKind::Rename => {
                        let src = work_dir.join(&job.target);
                        let safe = crate::extract::sanitize_entry_path(&job.target, work_dir)?;
                        if safe != src {
                            tokio::fs::rename(&src, &safe)
                                .await
                                .map_err(|e| Error::Internal(e.to_string()))?;
                        }
                    }
                    StepKind::InspectMedia => {
                        let p = work_dir.join(&job.target);
                        let result = tokio::process::Command::new("ffprobe")
                            .arg("-v")
                            .arg("error")
                            .arg("-show_entries")
                            .arg("format=duration")
                            .arg("-of")
                            .arg("json")
                            .arg(&p)
                            .output()
                            .await;
                        if result.is_err() {
                            // ffprobe absent: skip silently, do NOT fail.
                        }
                        // On success, mark completed regardless of content.
                    }
                    StepKind::Move | StepKind::Copy | StepKind::Hardlink => {
                        let src = work_dir.join(&job.target);
                        let file_name = std::path::Path::new(&job.target)
                            .file_name()
                            .and_then(|f| f.to_str())
                            .unwrap_or(job.target.as_str());
                        let dst = dest_dir.join(file_name);
                        match kind {
                            StepKind::Move => {
                                tokio::fs::rename(&src, &dst)
                                    .await
                                    .map_err(|e| Error::Internal(e.to_string()))?;
                            }
                            StepKind::Copy => {
                                tokio::fs::copy(&src, &dst)
                                    .await
                                    .map_err(|e| Error::Internal(e.to_string()))?;
                            }
                            StepKind::Hardlink => {
                                tokio::fs::hard_link(&src, &dst)
                                    .await
                                    .map_err(|e| Error::Internal(e.to_string()))?;
                            }
                            _ => unreachable!(),
                        }
                        // is_within canonicalizes, so the destination must exist first.
                        if !crate::pathutil::is_within(dest_dir, &dst) {
                            return Err(Error::UnsafePath);
                        }
                    }
                    StepKind::Cleanup => {
                        let extracted_before = job.steps[..i].iter().any(|s| {
                            s.kind == StepKind::Extract && s.state == StepState::Completed
                        });
                        if extracted_before {
                            let src = work_dir.join(&job.target);
                            let _ = tokio::fs::remove_file(&src).await;
                        }
                    }
                    StepKind::RunInstaller => {
                        if self.installer_policy != InstallerPolicy::Allow {
                            return Err(Error::ProcessLaunchDenied);
                        }
                        let exe = work_dir.join(&job.target);
                        std::process::Command::new(exe)
                            .spawn()
                            .map_err(|e| Error::Internal(e.to_string()))?;
                    }
                    StepKind::CustomHook => {
                        return Err(Error::ProcessLaunchDenied);
                    }
                }
                Ok(())
            })
            .await;

            match outcome {
                Ok(()) => {
                    job.steps[i].state = StepState::Completed;
                    job.steps[i].completed_at = Some(Utc::now());
                }
                Err(e) => {
                    job.steps[i].state = StepState::Failed;
                    job.steps[i].error = Some(e.to_string());
                    job.state = JobState::Failed;
                    job.error = Some(e.to_string());
                    return Err(e);
                }
            }
        }
        job.state = JobState::Completed;
        job.updated_at = Utc::now();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agpeer_jobs::Step;

    fn temp_dir() -> std::path::PathBuf {
        std::env::temp_dir().join(uuid::Uuid::new_v4().to_string())
    }

    fn make_step(index: u32, kind: StepKind, state: StepState) -> Step {
        Step {
            index,
            kind,
            state,
            started_at: None,
            completed_at: None,
            error: None,
        }
    }

    fn make_job(target: &str, steps: Vec<Step>) -> Job {
        Job {
            id: uuid::Uuid::new_v4(),
            transfer_id: agpeer_common::TransferId::new(),
            target: target.to_string(),
            state: JobState::Pending,
            steps,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            error: None,
        }
    }

    #[tokio::test]
    async fn pipeline_completes_verify_rename_move() {
        let work = temp_dir();
        let dest = temp_dir();
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        let target = "sample.bin";
        std::fs::write(work.join(target), b"payload").unwrap();

        let mut job = make_job(
            target,
            vec![
                make_step(0, StepKind::Verify, StepState::Pending),
                make_step(1, StepKind::Rename, StepState::Pending),
                make_step(2, StepKind::Move, StepState::Pending),
            ],
        );
        let pipeline = Pipeline::new(Arc::new(crate::extract::SevenZipExtractor::default()));
        pipeline.run(&mut job, &work, &dest).await.unwrap();

        assert!(job.steps.iter().all(|s| s.state == StepState::Completed));
        assert_eq!(job.state, JobState::Completed);
        assert!(dest.join(target).exists());
        assert!(!work.join(target).exists());

        std::fs::remove_dir_all(&work).ok();
        std::fs::remove_dir_all(&dest).ok();
    }

    #[tokio::test]
    async fn verify_fails_on_missing_target() {
        let work = temp_dir();
        let dest = temp_dir();
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(&dest).unwrap();

        let mut job = make_job(
            "missing.bin",
            vec![make_step(0, StepKind::Verify, StepState::Pending)],
        );
        let pipeline = Pipeline::new(Arc::new(crate::extract::SevenZipExtractor::default()));
        let err = pipeline.run(&mut job, &work, &dest).await.unwrap_err();

        assert_eq!(err.code(), "ExtractionFailed");
        assert_eq!(job.steps[0].state, StepState::Failed);
        assert_eq!(job.state, JobState::Failed);

        std::fs::remove_dir_all(&work).ok();
        std::fs::remove_dir_all(&dest).ok();
    }

    #[test]
    fn resume_from_skips_completed() {
        let job = make_job(
            "x.bin",
            vec![
                make_step(0, StepKind::Verify, StepState::Completed),
                make_step(1, StepKind::Rename, StepState::Pending),
            ],
        );
        assert_eq!(resume_from(&job), Some(1));
    }
}
