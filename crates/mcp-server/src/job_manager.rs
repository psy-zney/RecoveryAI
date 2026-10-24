//! Async Job Manager — quản lý tác vụ quét đĩa chạy ngầm.
//!
//! Khi AI gọi tool quét đĩa (scan signatures, carving...),
//! tác vụ được đẩy vào background job queue để tránh MCP timeout.
//! AI kiểm tra tiến trình bằng job_id.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, oneshot};
use uuid::Uuid;

/// Trạng thái của một job
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
pub enum JobStatus {
    /// Đang chạy, kèm phần trăm hoàn thành
    Running { progress_percent: f64, message: String },
    /// Hoàn thành thành công
    Completed { result: serde_json::Value },
    /// Thất bại
    Failed { error: String },
    /// Đã bị huỷ
    Cancelled,
}

/// Thông tin một background job
#[derive(Debug, Clone, Serialize)]
pub struct JobInfo {
    pub job_id: String,
    pub job_type: String,
    pub status: JobStatus,
    pub created_at: String,
    pub updated_at: String,
}

/// Quản lý các background job
pub struct JobManager {
    jobs: Arc<Mutex<HashMap<String, JobInfo>>>,
    /// Kênh gửi tín hiệu huỷ job
    cancel_senders: Arc<Mutex<HashMap<String, oneshot::Sender<()>>>>,
}

#[allow(dead_code)]
impl JobManager {
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(Mutex::new(HashMap::new())),
            cancel_senders: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Tạo job mới và trả về job_id.
    pub async fn create_job(&self, job_type: &str) -> (String, oneshot::Receiver<()>) {
        let job_id = Uuid::new_v4().to_string();
        let now = chrono_now();

        let (cancel_tx, cancel_rx) = oneshot::channel();

        let info = JobInfo {
            job_id: job_id.clone(),
            job_type: job_type.to_string(),
            status: JobStatus::Running {
                progress_percent: 0.0,
                message: "Starting...".into(),
            },
            created_at: now.clone(),
            updated_at: now,
        };

        self.jobs.lock().await.insert(job_id.clone(), info);
        self.cancel_senders
            .lock()
            .await
            .insert(job_id.clone(), cancel_tx);

        (job_id, cancel_rx)
    }

    /// Cập nhật tiến trình job.
    pub async fn update_progress(&self, job_id: &str, percent: f64, message: &str) {
        if let Some(job) = self.jobs.lock().await.get_mut(job_id) {
            job.status = JobStatus::Running {
                progress_percent: percent,
                message: message.to_string(),
            };
            job.updated_at = chrono_now();
        }
    }

    /// Đánh dấu job hoàn thành.
    pub async fn complete_job(&self, job_id: &str, result: serde_json::Value) {
        if let Some(job) = self.jobs.lock().await.get_mut(job_id) {
            job.status = JobStatus::Completed { result };
            job.updated_at = chrono_now();
        }
        self.cancel_senders.lock().await.remove(job_id);
    }

    /// Đánh dấu job thất bại.
    pub async fn fail_job(&self, job_id: &str, error: &str) {
        if let Some(job) = self.jobs.lock().await.get_mut(job_id) {
            job.status = JobStatus::Failed {
                error: error.to_string(),
            };
            job.updated_at = chrono_now();
        }
        self.cancel_senders.lock().await.remove(job_id);
    }

    /// Huỷ job đang chạy.
    pub async fn cancel_job(&self, job_id: &str) -> bool {
        if let Some(sender) = self.cancel_senders.lock().await.remove(job_id) {
            let _ = sender.send(());
            if let Some(job) = self.jobs.lock().await.get_mut(job_id) {
                job.status = JobStatus::Cancelled;
                job.updated_at = chrono_now();
            }
            true
        } else {
            false
        }
    }

    /// Lấy thông tin một job.
    pub async fn get_job(&self, job_id: &str) -> Option<JobInfo> {
        self.jobs.lock().await.get(job_id).cloned()
    }

    /// Liệt kê tất cả jobs.
    pub async fn list_jobs(&self) -> Vec<JobInfo> {
        self.jobs.lock().await.values().cloned().collect()
    }

    /// Lấy Arc clone để chia sẻ giữa các task.
    pub fn jobs_ref(&self) -> Arc<Mutex<HashMap<String, JobInfo>>> {
        Arc::clone(&self.jobs)
    }
}

impl Default for JobManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Lấy timestamp hiện tại dạng ISO 8601 (đơn giản, không cần chrono crate)
fn chrono_now() -> String {
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}s since epoch", duration.as_secs())
}
