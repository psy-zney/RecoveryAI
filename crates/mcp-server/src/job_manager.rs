//! Bounded job registry; worker threads never block the async runtime on I/O.
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use uuid::Uuid;

#[derive(Debug, Clone, Copy)]
pub enum JobType {
    DataExtraction,
}
impl JobType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DataExtraction => "DATA_EXTRACTION",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum JobStatus {
    Running {
        progress_percent: f64,
        message: String,
        sectors_processed: u64,
        items_found: u64,
        bytes_written: u64,
    },
    Completed {
        result: Value,
    },
    Failed {
        error: String,
    },
    Cancelled,
}
#[derive(Debug, Clone, Serialize)]
pub struct JobInfo {
    pub job_id: String,
    pub job_type: String,
    pub status: JobStatus,
}
struct Job {
    info: JobInfo,
    cancel: Arc<AtomicBool>,
    hits_path: Option<std::path::PathBuf>,
}
#[derive(Default)]
pub struct JobManager {
    jobs: Mutex<HashMap<String, Job>>,
}
impl JobManager {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn create_job(&self, kind: &str) -> anyhow::Result<(String, Arc<AtomicBool>)> {
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| anyhow::anyhow!("Job registry poisoned"))?;
        let running = jobs
            .values()
            .filter(|j| matches!(j.info.status, JobStatus::Running { .. }))
            .count();
        anyhow::ensure!(
            running < 4,
            "At most four background jobs may run concurrently"
        );
        if jobs.len() >= 128 {
            if let Some(key) = jobs
                .iter()
                .find(|(_, j)| !matches!(j.info.status, JobStatus::Running { .. }))
                .map(|(k, _)| k.clone())
            {
                jobs.remove(&key);
            }
        }
        let id = Uuid::new_v4().to_string();
        let cancel = Arc::new(AtomicBool::new(false));
        jobs.insert(
            id.clone(),
            Job {
                info: JobInfo {
                    job_id: id.clone(),
                    job_type: kind.into(),
                    status: JobStatus::Running {
                        progress_percent: 0.0,
                        message: "Queued".into(),
                        sectors_processed: 0,
                        items_found: 0,
                        bytes_written: 0,
                    },
                },
                cancel: cancel.clone(),
                hits_path: None,
            },
        );
        Ok((id, cancel))
    }
    pub fn set_hits_path(&self, id: &str, path: std::path::PathBuf) -> anyhow::Result<()> {
        self.jobs
            .lock()
            .map_err(|_| anyhow::anyhow!("Job registry poisoned"))?
            .get_mut(id)
            .ok_or_else(|| anyhow::anyhow!("Job not found"))?
            .hits_path = Some(path);
        Ok(())
    }
    pub fn hits_path(&self, id: &str) -> anyhow::Result<std::path::PathBuf> {
        self.jobs
            .lock()
            .map_err(|_| anyhow::anyhow!("Job registry poisoned"))?
            .get(id)
            .and_then(|job| job.hits_path.clone())
            .ok_or_else(|| anyhow::anyhow!("Job has no stored hit list"))
    }
    pub fn progress(&self, id: &str, done: u64, total: u64, found: u64) {
        if let Ok(mut jobs) = self.jobs.lock() {
            if let Some(job) = jobs.get_mut(id) {
                if matches!(job.info.status, JobStatus::Running { .. }) {
                    job.info.status = JobStatus::Running {
                        progress_percent: if total == 0 {
                            0.0
                        } else {
                            100.0 * done as f64 / total as f64
                        },
                        message: "Processing".into(),
                        sectors_processed: done,
                        items_found: found,
                        bytes_written: 0,
                    };
                }
            }
        }
    }
    pub fn extraction_progress(&self, id: &str, bytes: u64, total: Option<u64>) {
        if let Ok(mut jobs) = self.jobs.lock() {
            if let Some(job) = jobs.get_mut(id) {
                if matches!(job.info.status, JobStatus::Running { .. }) {
                    job.info.status = JobStatus::Running {
                        progress_percent: total
                            .filter(|n| *n > 0)
                            .map_or(0.0, |n| 100.0 * bytes as f64 / n as f64),
                        message: "Extracting".into(),
                        sectors_processed: 0,
                        items_found: 0,
                        bytes_written: bytes,
                    };
                }
            }
        }
    }
    pub fn finish(&self, id: &str, result: anyhow::Result<Value>) {
        if let Ok(mut jobs) = self.jobs.lock() {
            if let Some(job) = jobs.get_mut(id) {
                // Committed output wins cancellation racing with the final commit.
                job.info.status = match result {
                    Ok(result) => JobStatus::Completed { result },
                    Err(_) if job.cancel.load(Ordering::Acquire) => JobStatus::Cancelled,
                    Err(error) => JobStatus::Failed {
                        error: error.to_string().chars().take(1024).collect(),
                    },
                };
            }
        }
    }
    pub fn cancel_job(&self, id: &str) -> anyhow::Result<()> {
        let jobs = self
            .jobs
            .lock()
            .map_err(|_| anyhow::anyhow!("Job registry poisoned"))?;
        let job = jobs
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("Job not found"))?;
        anyhow::ensure!(
            matches!(job.info.status, JobStatus::Running { .. }),
            "Job has already finished"
        );
        job.cancel.store(true, Ordering::Release);
        Ok(())
    }
    pub fn get_job(&self, id: &str) -> anyhow::Result<JobInfo> {
        self.jobs
            .lock()
            .map_err(|_| anyhow::anyhow!("Job registry poisoned"))?
            .get(id)
            .map(|j| j.info.clone())
            .ok_or_else(|| anyhow::anyhow!("Job not found"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_racing_with_committed_success_does_not_hide_output() {
        let jobs = JobManager::new();
        let (id, _) = jobs.create_job("clone").unwrap();
        jobs.cancel_job(&id).unwrap();
        jobs.finish(&id, Ok(serde_json::json!({"status":"cloned"})));
        assert!(matches!(
            jobs.get_job(&id).unwrap().status,
            JobStatus::Completed { .. }
        ));
    }
    #[test]
    fn cancellation_is_acknowledged_by_worker_and_terminal_progress_is_immutable() {
        let jobs = JobManager::new();
        let (id, flag) = jobs.create_job("scan").unwrap();
        jobs.cancel_job(&id).unwrap();
        assert!(flag.load(Ordering::Acquire));
        jobs.finish(&id, Err(anyhow::anyhow!("Job cancelled")));
        jobs.progress(&id, 1, 2, 0);
        assert!(matches!(
            jobs.get_job(&id).unwrap().status,
            JobStatus::Cancelled
        ));
    }
}
