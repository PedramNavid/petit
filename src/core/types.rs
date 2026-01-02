//! Core identifier types for the orchestrator.
//!
//! These types provide type-safe identifiers for tasks, jobs, runs, and DAGs.

use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Unique identifier for a task within a DAG.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(String);

/// Unique identifier for a job.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JobId(String);

/// Unique identifier for a job run (execution instance).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RunId(Uuid);

/// Unique identifier for a DAG.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DagId(String);

impl TaskId {
    /// Create a new TaskId from a string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Get the underlying string value.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl JobId {
    /// Create a new JobId from a string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Get the underlying string value.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl RunId {
    /// Generate a new random RunId.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create a RunId from an existing UUID.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Get the underlying UUID.
    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

impl DagId {
    /// Create a new DagId from a string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Get the underlying string value.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for DagId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_id_creation() {
        let task_id = TaskId::new("extract_data");
        assert_eq!(task_id.as_str(), "extract_data");
    }

    #[test]
    fn test_task_id_display() {
        let task_id = TaskId::new("transform");
        assert_eq!(format!("{}", task_id), "transform");
    }

    #[test]
    fn test_task_id_equality() {
        let id1 = TaskId::new("task_a");
        let id2 = TaskId::new("task_a");
        let id3 = TaskId::new("task_b");

        assert_eq!(id1, id2);
        assert_ne!(id1, id3);
    }

    #[test]
    fn test_job_id_creation() {
        let job_id = JobId::new("daily_etl");
        assert_eq!(job_id.as_str(), "daily_etl");
    }

    #[test]
    fn test_run_id_is_unique() {
        let run1 = RunId::new();
        let run2 = RunId::new();

        assert_ne!(run1, run2);
    }

    #[test]
    fn test_run_id_from_uuid() {
        let uuid = Uuid::new_v4();
        let run_id = RunId::from_uuid(uuid);

        assert_eq!(run_id.as_uuid(), &uuid);
    }

    #[test]
    fn test_dag_id_creation() {
        let dag_id = DagId::new("pipeline_v1");
        assert_eq!(dag_id.as_str(), "pipeline_v1");
    }

    #[test]
    fn test_ids_are_hashable() {
        use std::collections::HashSet;

        let mut task_ids: HashSet<TaskId> = HashSet::new();
        task_ids.insert(TaskId::new("task1"));
        task_ids.insert(TaskId::new("task2"));
        task_ids.insert(TaskId::new("task1")); // duplicate

        assert_eq!(task_ids.len(), 2);
    }
}
