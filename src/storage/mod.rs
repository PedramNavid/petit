//! Storage abstraction for persisting jobs, runs, and task state.
//!
//! This module provides a trait-based storage abstraction with
//! pluggable backends (in-memory, SQLite, etc.).

mod memory;
#[cfg(any(feature = "sqlite", test))]
mod sqlite;

pub use memory::InMemoryStorage;
#[cfg(any(feature = "sqlite", test))]
pub use sqlite::SqliteStorage;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};
use thiserror::Error;

use crate::core::types::{DagId, JobId, RunId, TaskId};

/// Errors that can occur during storage operations.
#[derive(Debug, Error)]
pub enum StorageError {
    /// The requested item was not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// A duplicate key was detected.
    #[error("duplicate key: {0}")]
    DuplicateKey(String),

    /// Storage lock was poisoned.
    #[error("storage lock poisoned")]
    LockPoisoned,

    /// Serialization/deserialization error.
    #[error("serialization error: {0}")]
    SerializationError(String),

    /// Generic storage error.
    #[error("storage error: {0}")]
    Other(String),
}

/// Status of a job run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunStatus {
    /// Run is pending execution.
    Pending,
    /// Run is currently executing.
    Running,
    /// Run completed successfully.
    Completed,
    /// Run failed with errors.
    Failed,
    /// Run was interrupted (e.g., scheduler restart).
    Interrupted,
    /// Run was cancelled by user.
    Cancelled,
}

/// Status of a task within a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskRunStatus {
    /// Task is pending execution.
    Pending,
    /// Task is currently running.
    Running,
    /// Task completed successfully.
    Completed,
    /// Task failed.
    Failed,
    /// Task was skipped (e.g., upstream failure).
    Skipped,
}

/// Stored job definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredJob {
    /// Unique job identifier.
    pub id: JobId,
    /// Human-readable job name.
    pub name: String,
    /// Associated DAG identifier.
    pub dag_id: DagId,
    /// Optional cron schedule.
    pub schedule: Option<String>,
    /// When the job was created.
    pub created_at: SystemTime,
    /// When the job was last updated.
    pub updated_at: SystemTime,
    /// Whether the job is enabled.
    pub enabled: bool,
}

impl StoredJob {
    /// Create a new stored job.
    pub fn new(id: JobId, name: impl Into<String>, dag_id: DagId) -> Self {
        let now = SystemTime::now();
        Self {
            id,
            name: name.into(),
            dag_id,
            schedule: None,
            created_at: now,
            updated_at: now,
            enabled: true,
        }
    }

    /// Set the schedule.
    pub fn with_schedule(mut self, schedule: impl Into<String>) -> Self {
        self.schedule = Some(schedule.into());
        self
    }

    /// Set enabled status.
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// Stored job run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredRun {
    /// Unique run identifier.
    pub id: RunId,
    /// Parent job identifier.
    pub job_id: JobId,
    /// Run status.
    pub status: RunStatus,
    /// When the run started.
    pub started_at: SystemTime,
    /// When the run ended (if finished).
    pub ended_at: Option<SystemTime>,
    /// Total duration (if finished).
    pub duration: Option<Duration>,
    /// Error message (if failed).
    pub error: Option<String>,
}

impl StoredRun {
    /// Create a new stored run.
    pub fn new(id: RunId, job_id: JobId) -> Self {
        Self {
            id,
            job_id,
            status: RunStatus::Pending,
            started_at: SystemTime::now(),
            ended_at: None,
            duration: None,
            error: None,
        }
    }

    /// Mark the run as running.
    pub fn mark_running(&mut self) {
        self.status = RunStatus::Running;
        self.started_at = SystemTime::now();
    }

    /// Mark the run as completed.
    pub fn mark_completed(&mut self) {
        self.status = RunStatus::Completed;
        self.ended_at = Some(SystemTime::now());
        self.duration = self.ended_at.unwrap().duration_since(self.started_at).ok();
    }

    /// Mark the run as failed.
    pub fn mark_failed(&mut self, error: impl Into<String>) {
        self.status = RunStatus::Failed;
        self.ended_at = Some(SystemTime::now());
        self.duration = self.ended_at.unwrap().duration_since(self.started_at).ok();
        self.error = Some(error.into());
    }

    /// Mark the run as interrupted.
    pub fn mark_interrupted(&mut self) {
        self.status = RunStatus::Interrupted;
        self.ended_at = Some(SystemTime::now());
        self.duration = self.ended_at.unwrap().duration_since(self.started_at).ok();
    }

    /// Mark the run as cancelled.
    pub fn mark_cancelled(&mut self) {
        self.status = RunStatus::Cancelled;
        self.ended_at = Some(SystemTime::now());
        self.duration = self.ended_at.unwrap().duration_since(self.started_at).ok();
    }
}

/// Stored task state within a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredTaskState {
    /// Task identifier.
    pub task_id: TaskId,
    /// Parent run identifier.
    pub run_id: RunId,
    /// Task status.
    pub status: TaskRunStatus,
    /// Number of attempts made.
    pub attempts: u32,
    /// When the task started.
    pub started_at: Option<SystemTime>,
    /// When the task ended.
    pub ended_at: Option<SystemTime>,
    /// Task duration.
    pub duration: Option<Duration>,
    /// Error message (if failed).
    pub error: Option<String>,
}

impl StoredTaskState {
    /// Create a new stored task state.
    pub fn new(task_id: TaskId, run_id: RunId) -> Self {
        Self {
            task_id,
            run_id,
            status: TaskRunStatus::Pending,
            attempts: 0,
            started_at: None,
            ended_at: None,
            duration: None,
            error: None,
        }
    }

    /// Mark the task as running.
    pub fn mark_running(&mut self) {
        self.status = TaskRunStatus::Running;
        self.started_at = Some(SystemTime::now());
        self.attempts += 1;
    }

    /// Mark the task as completed.
    pub fn mark_completed(&mut self) {
        self.status = TaskRunStatus::Completed;
        self.ended_at = Some(SystemTime::now());
        if let Some(started) = self.started_at {
            self.duration = self.ended_at.unwrap().duration_since(started).ok();
        }
    }

    /// Mark the task as failed.
    pub fn mark_failed(&mut self, error: impl Into<String>) {
        self.status = TaskRunStatus::Failed;
        self.ended_at = Some(SystemTime::now());
        if let Some(started) = self.started_at {
            self.duration = self.ended_at.unwrap().duration_since(started).ok();
        }
        self.error = Some(error.into());
    }

    /// Mark the task as skipped.
    pub fn mark_skipped(&mut self) {
        self.status = TaskRunStatus::Skipped;
    }
}

/// Storage trait for persisting orchestrator state.
#[async_trait]
pub trait Storage: Send + Sync {
    // Job operations

    /// Save a job definition.
    async fn save_job(&self, job: StoredJob) -> Result<(), StorageError>;

    /// Get a job by ID.
    async fn get_job(&self, id: &JobId) -> Result<StoredJob, StorageError>;

    /// List all jobs.
    async fn list_jobs(&self) -> Result<Vec<StoredJob>, StorageError>;

    /// Delete a job by ID.
    async fn delete_job(&self, id: &JobId) -> Result<(), StorageError>;

    // Run operations

    /// Save a job run.
    async fn save_run(&self, run: StoredRun) -> Result<(), StorageError>;

    /// Get a run by ID.
    async fn get_run(&self, id: &RunId) -> Result<StoredRun, StorageError>;

    /// List runs for a job, ordered by start time descending.
    /// Returns at most `limit` runs.
    async fn list_runs(&self, job_id: &JobId, limit: usize)
    -> Result<Vec<StoredRun>, StorageError>;

    /// Update run status.
    async fn update_run(&self, run: StoredRun) -> Result<(), StorageError>;

    /// Get all incomplete runs (Pending or Running status).
    async fn get_incomplete_runs(&self) -> Result<Vec<StoredRun>, StorageError>;

    /// Mark a run as interrupted.
    async fn mark_run_interrupted(&self, id: &RunId) -> Result<(), StorageError>;

    // Task state operations

    /// Save task state.
    async fn save_task_state(&self, state: StoredTaskState) -> Result<(), StorageError>;

    /// Get task state.
    async fn get_task_state(
        &self,
        run_id: &RunId,
        task_id: &TaskId,
    ) -> Result<StoredTaskState, StorageError>;

    /// List all task states for a run.
    async fn list_task_states(&self, run_id: &RunId) -> Result<Vec<StoredTaskState>, StorageError>;

    /// Update task state.
    async fn update_task_state(&self, state: StoredTaskState) -> Result<(), StorageError>;
}
