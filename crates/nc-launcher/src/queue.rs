//! Download queue state: one active installation, FIFO pending jobs.
//!
//! Pure bookkeeping: the worker that runs jobs lives in the application
//! layer. Every mutation returns enough information for the caller to emit
//! a snapshot to the UI.

use std::collections::VecDeque;

use serde::Serialize;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum JobKind {
    /// Install or repair a game version (and its Java runtime).
    Version,
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: u64,
    pub kind: JobKind,
    pub target: String,
    pub queued_at: i64,
    pub cancel: CancellationToken,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    pub id: u64,
    pub kind: JobKind,
    pub target: String,
    pub queued_at: i64,
}

impl From<&Job> for JobView {
    fn from(j: &Job) -> Self {
        Self {
            id: j.id,
            kind: j.kind,
            target: j.target.clone(),
            queued_at: j.queued_at,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueSnapshot {
    pub active: Option<JobView>,
    pub pending: Vec<JobView>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Enqueued {
    Added(u64),
    /// The same target is already active or pending.
    AlreadyQueued(u64),
}

#[derive(Debug, Default)]
pub struct Queue {
    next_id: u64,
    active: Option<Job>,
    pending: VecDeque<Job>,
}

impl Queue {
    pub fn enqueue(&mut self, kind: JobKind, target: &str, now: i64) -> Enqueued {
        if let Some(j) = self
            .active
            .iter()
            .chain(self.pending.iter())
            .find(|j| j.kind == kind && j.target == target)
        {
            return Enqueued::AlreadyQueued(j.id);
        }
        self.next_id += 1;
        self.pending.push_back(Job {
            id: self.next_id,
            kind,
            target: target.to_owned(),
            queued_at: now,
            cancel: CancellationToken::new(),
        });
        Enqueued::Added(self.next_id)
    }

    /// Move the next pending job to active, if nothing is running.
    pub fn start_next(&mut self) -> Option<Job> {
        if self.active.is_some() {
            return None;
        }
        self.active = self.pending.pop_front();
        self.active.clone()
    }

    /// Mark the active job finished.
    pub fn finish(&mut self, id: u64) -> Option<Job> {
        if self.active.as_ref().is_some_and(|j| j.id == id) {
            self.active.take()
        } else {
            None
        }
    }

    /// Cancel the active job or remove a pending one. Returns whether found.
    pub fn cancel(&mut self, id: u64) -> bool {
        if let Some(a) = &self.active
            && a.id == id
        {
            a.cancel.cancel();
            return true;
        }
        let before = self.pending.len();
        self.pending.retain(|j| j.id != id);
        before != self.pending.len()
    }

    pub fn is_busy_with(&self, kind: JobKind, target: &str) -> bool {
        self.active
            .iter()
            .chain(self.pending.iter())
            .any(|j| j.kind == kind && j.target == target)
    }

    pub fn active_target(&self) -> Option<&str> {
        self.active.as_ref().map(|j| j.target.as_str())
    }

    pub fn snapshot(&self) -> QueueSnapshot {
        QueueSnapshot {
            active: self.active.as_ref().map(Into::into),
            pending: self.pending.iter().map(Into::into).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fifo_one_active_and_dedup() {
        let mut q = Queue::default();
        let Enqueued::Added(a) = q.enqueue(JobKind::Version, "1.21.1", 1) else {
            panic!()
        };
        let Enqueued::Added(b) = q.enqueue(JobKind::Version, "26.3", 2) else {
            panic!()
        };
        assert_eq!(
            q.enqueue(JobKind::Version, "1.21.1", 3),
            Enqueued::AlreadyQueued(a)
        );

        let first = q.start_next().unwrap();
        assert_eq!(first.target, "1.21.1");
        assert!(q.start_next().is_none(), "only one active job");
        assert_eq!(
            q.enqueue(JobKind::Version, "1.21.1", 4),
            Enqueued::AlreadyQueued(a),
            "active counts too"
        );

        assert!(q.finish(a).is_some());
        assert!(q.finish(a).is_none());
        assert_eq!(q.start_next().unwrap().id, b);
        assert!(q.is_busy_with(JobKind::Version, "26.3"));
    }

    #[test]
    fn cancel_active_and_remove_pending() {
        let mut q = Queue::default();
        q.enqueue(JobKind::Version, "a", 0);
        let Enqueued::Added(b) = q.enqueue(JobKind::Version, "b", 0) else {
            panic!()
        };
        let active = q.start_next().unwrap();
        assert!(q.cancel(b));
        assert!(q.snapshot().pending.is_empty());
        assert!(q.cancel(active.id));
        assert!(
            active.cancel.is_cancelled(),
            "token shared with the running job"
        );
        assert!(!q.cancel(999));
    }
}
