//! Scheduler type definitions.
//!
//! This module contains error types, state enums, and command types for the scheduler.

use crate::core::types::{JobId, RunId};
use crate::storage::StorageError;
use thiserror::Error;
use tokio::sync::oneshot;

/// Errors that can occur in the scheduler.
#[derive(Debug, Error)]
pub enum SchedulerError {
    /// Job not found.
    #[error("job not found: {0}")]
    JobNotFound(String),

    /// Storage error.
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    /// Channel error.
    #[error("channel error: {0}")]
    ChannelError(String),

    /// Job dependency not satisfied.
    #[error("job dependency not satisfied: {0}")]
    DependencyNotSatisfied(String),

    /// Max concurrent runs exceeded.
    #[error("max concurrent runs exceeded for job: {0}")]
    MaxConcurrentRunsExceeded(String),
}

/// State of the scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerState {
    /// Scheduler is stopped.
    Stopped,
    /// Scheduler is running.
    Running,
    /// Scheduler is paused.
    Paused,
}

/// Commands that can be sent to the scheduler.
pub(crate) enum SchedulerCommand {
    /// Trigger a job manually.
    Trigger {
        job_id: JobId,
        response: oneshot::Sender<Result<RunId, SchedulerError>>,
    },
    /// Pause the scheduler.
    Pause { response: oneshot::Sender<()> },
    /// Resume the scheduler.
    Resume { response: oneshot::Sender<()> },
    /// Shutdown the scheduler.
    Shutdown { response: oneshot::Sender<()> },
}
