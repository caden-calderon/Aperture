//! Async compression queue contract.
//!
//! This is intentionally runtime-agnostic and thread-safe so worker execution
//! can be added in later checkpoints without changing queue semantics.

use std::collections::VecDeque;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::provider::CompressionTarget;

/// Queue status for an individual compression job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompressionJobStatus {
    Pending,
    Completed,
    Failed,
}

/// Compression work item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressionJob {
    pub id: String,
    pub block_id: String,
    pub target: CompressionTargetWire,
    pub preserve_keys: Vec<String>,
    pub source_tokens: u32,
    pub status: CompressionJobStatus,
    pub output_tokens: Option<u32>,
    pub error: Option<String>,
}

impl CompressionJob {
    pub fn new(
        id: String,
        block_id: String,
        target: CompressionTarget,
        preserve_keys: Vec<String>,
        source_tokens: u32,
    ) -> Self {
        Self {
            id,
            block_id,
            target: target.into(),
            preserve_keys,
            source_tokens,
            status: CompressionJobStatus::Pending,
            output_tokens: None,
            error: None,
        }
    }
}

/// Serde-friendly target representation for queue persistence/IPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompressionTargetWire {
    Summarized,
    Minimal,
}

impl From<CompressionTarget> for CompressionTargetWire {
    fn from(value: CompressionTarget) -> Self {
        match value {
            CompressionTarget::Summarized => Self::Summarized,
            CompressionTarget::Minimal => Self::Minimal,
        }
    }
}

/// In-memory queue with pending and recently completed jobs.
#[derive(Debug, Default)]
pub struct CompressionQueue {
    pending: Mutex<VecDeque<CompressionJob>>,
    completed: Mutex<VecDeque<CompressionJob>>,
}

impl CompressionQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enqueue a new compression job.
    pub fn enqueue(&self, job: CompressionJob) {
        let mut guard = self.pending.lock().expect("pending queue lock");
        guard.push_back(job);
    }

    /// Dequeue the next pending job.
    pub fn dequeue(&self) -> Option<CompressionJob> {
        let mut guard = self.pending.lock().expect("pending queue lock");
        guard.pop_front()
    }

    /// Mark a job completed and store completion metadata.
    pub fn mark_completed(&self, mut job: CompressionJob, output_tokens: u32) {
        job.status = CompressionJobStatus::Completed;
        job.output_tokens = Some(output_tokens);
        job.error = None;
        self.completed
            .lock()
            .expect("completed queue lock")
            .push_back(job);
    }

    /// Mark a job failed and record error text.
    pub fn mark_failed(&self, mut job: CompressionJob, error: impl Into<String>) {
        job.status = CompressionJobStatus::Failed;
        job.output_tokens = None;
        job.error = Some(error.into());
        self.completed
            .lock()
            .expect("completed queue lock")
            .push_back(job);
    }

    pub fn pending_len(&self) -> usize {
        self.pending.lock().expect("pending queue lock").len()
    }

    pub fn completed_snapshot(&self) -> Vec<CompressionJob> {
        self.completed
            .lock()
            .expect("completed queue lock")
            .iter()
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_job(id: &str) -> CompressionJob {
        CompressionJob::new(
            id.to_string(),
            "b1".to_string(),
            CompressionTarget::Summarized,
            vec!["error".to_string()],
            900,
        )
    }

    #[test]
    fn test_queue_enqueue_dequeue_fifo() {
        let queue = CompressionQueue::new();
        queue.enqueue(make_job("j1"));
        queue.enqueue(make_job("j2"));

        let first = queue.dequeue().expect("first job");
        let second = queue.dequeue().expect("second job");
        assert_eq!(first.id, "j1");
        assert_eq!(second.id, "j2");
        assert!(queue.dequeue().is_none());
    }

    #[test]
    fn test_mark_completed_records_output_tokens() {
        let queue = CompressionQueue::new();
        let job = make_job("j1");
        queue.mark_completed(job, 80);

        let completed = queue.completed_snapshot();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].status, CompressionJobStatus::Completed);
        assert_eq!(completed[0].output_tokens, Some(80));
        assert!(completed[0].error.is_none());
    }

    #[test]
    fn test_mark_failed_records_error() {
        let queue = CompressionQueue::new();
        let job = make_job("j1");
        queue.mark_failed(job, "timeout");

        let completed = queue.completed_snapshot();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].status, CompressionJobStatus::Failed);
        assert_eq!(completed[0].error.as_deref(), Some("timeout"));
        assert!(completed[0].output_tokens.is_none());
    }
}
