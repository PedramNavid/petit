//! In-memory storage implementation.
//!
//! Provides a thread-safe in-memory backend for testing and development.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::RwLock;

use super::{RunStatus, Storage, StorageError, StoredJob, StoredRun, StoredTaskState};

#[cfg(test)]
use super::TaskRunStatus;
use crate::core::types::{JobId, RunId, TaskId};

/// In-memory storage backend.
///
/// Thread-safe storage using RwLock for concurrent access.
/// Data is not persisted across restarts.
pub struct InMemoryStorage {
    jobs: RwLock<HashMap<JobId, StoredJob>>,
    runs: RwLock<HashMap<RunId, StoredRun>>,
    task_states: RwLock<HashMap<(RunId, TaskId), StoredTaskState>>,
}

impl InMemoryStorage {
    /// Create a new empty in-memory storage.
    pub fn new() -> Self {
        Self {
            jobs: RwLock::new(HashMap::new()),
            runs: RwLock::new(HashMap::new()),
            task_states: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Storage for InMemoryStorage {
    async fn save_job(&self, job: StoredJob) -> Result<(), StorageError> {
        let mut jobs = self.jobs.write().map_err(|_| StorageError::LockPoisoned)?;
        if jobs.contains_key(&job.id) {
            return Err(StorageError::DuplicateKey(format!("job: {}", job.id)));
        }
        jobs.insert(job.id.clone(), job);
        Ok(())
    }

    async fn upsert_job(&self, job: StoredJob) -> Result<(), StorageError> {
        let mut jobs = self.jobs.write().map_err(|_| StorageError::LockPoisoned)?;
        jobs.insert(job.id.clone(), job);
        Ok(())
    }

    async fn get_job(&self, id: &JobId) -> Result<StoredJob, StorageError> {
        let jobs = self.jobs.read().map_err(|_| StorageError::LockPoisoned)?;
        jobs.get(id)
            .cloned()
            .ok_or_else(|| StorageError::NotFound(format!("job: {}", id)))
    }

    async fn list_jobs(&self) -> Result<Vec<StoredJob>, StorageError> {
        let jobs = self.jobs.read().map_err(|_| StorageError::LockPoisoned)?;
        let mut result: Vec<_> = jobs.values().cloned().collect();
        result.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(result)
    }

    async fn delete_job(&self, id: &JobId) -> Result<(), StorageError> {
        let mut jobs = self.jobs.write().map_err(|_| StorageError::LockPoisoned)?;
        jobs.remove(id)
            .ok_or_else(|| StorageError::NotFound(format!("job: {}", id)))?;
        Ok(())
    }

    async fn save_run(&self, run: StoredRun) -> Result<(), StorageError> {
        let mut runs = self.runs.write().map_err(|_| StorageError::LockPoisoned)?;
        if runs.contains_key(&run.id) {
            return Err(StorageError::DuplicateKey(format!("run: {}", run.id)));
        }
        runs.insert(run.id.clone(), run);
        Ok(())
    }

    async fn get_run(&self, id: &RunId) -> Result<StoredRun, StorageError> {
        let runs = self.runs.read().map_err(|_| StorageError::LockPoisoned)?;
        runs.get(id)
            .cloned()
            .ok_or_else(|| StorageError::NotFound(format!("run: {}", id)))
    }

    async fn list_runs(
        &self,
        job_id: &JobId,
        limit: usize,
    ) -> Result<Vec<StoredRun>, StorageError> {
        let runs = self.runs.read().map_err(|_| StorageError::LockPoisoned)?;
        let mut result: Vec<_> = runs
            .values()
            .filter(|r| &r.job_id == job_id)
            .cloned()
            .collect();
        // Sort by started_at descending (most recent first)
        result.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        result.truncate(limit);
        Ok(result)
    }

    async fn update_run(&self, run: StoredRun) -> Result<(), StorageError> {
        let mut runs = self.runs.write().map_err(|_| StorageError::LockPoisoned)?;
        if !runs.contains_key(&run.id) {
            return Err(StorageError::NotFound(format!("run: {}", run.id)));
        }
        runs.insert(run.id.clone(), run);
        Ok(())
    }

    async fn get_incomplete_runs(&self) -> Result<Vec<StoredRun>, StorageError> {
        let runs = self.runs.read().map_err(|_| StorageError::LockPoisoned)?;
        let result: Vec<_> = runs
            .values()
            .filter(|r| matches!(r.status, RunStatus::Pending | RunStatus::Running))
            .cloned()
            .collect();
        Ok(result)
    }

    async fn mark_run_interrupted(&self, id: &RunId) -> Result<(), StorageError> {
        let mut runs = self.runs.write().map_err(|_| StorageError::LockPoisoned)?;
        let run = runs
            .get_mut(id)
            .ok_or_else(|| StorageError::NotFound(format!("run: {}", id)))?;
        run.mark_interrupted();
        Ok(())
    }

    async fn save_task_state(&self, state: StoredTaskState) -> Result<(), StorageError> {
        let mut states = self
            .task_states
            .write()
            .map_err(|_| StorageError::LockPoisoned)?;
        let key = (state.run_id.clone(), state.task_id.clone());
        if states.contains_key(&key) {
            return Err(StorageError::DuplicateKey(format!(
                "task_state: {}/{}",
                state.run_id, state.task_id
            )));
        }
        states.insert(key, state);
        Ok(())
    }

    async fn get_task_state(
        &self,
        run_id: &RunId,
        task_id: &TaskId,
    ) -> Result<StoredTaskState, StorageError> {
        let states = self
            .task_states
            .read()
            .map_err(|_| StorageError::LockPoisoned)?;
        let key = (run_id.clone(), task_id.clone());
        states
            .get(&key)
            .cloned()
            .ok_or_else(|| StorageError::NotFound(format!("task_state: {}/{}", run_id, task_id)))
    }

    async fn list_task_states(&self, run_id: &RunId) -> Result<Vec<StoredTaskState>, StorageError> {
        let states = self
            .task_states
            .read()
            .map_err(|_| StorageError::LockPoisoned)?;
        let result: Vec<_> = states
            .values()
            .filter(|s| &s.run_id == run_id)
            .cloned()
            .collect();
        Ok(result)
    }

    async fn update_task_state(&self, state: StoredTaskState) -> Result<(), StorageError> {
        let mut states = self
            .task_states
            .write()
            .map_err(|_| StorageError::LockPoisoned)?;
        let key = (state.run_id.clone(), state.task_id.clone());
        if !states.contains_key(&key) {
            return Err(StorageError::NotFound(format!(
                "task_state: {}/{}",
                state.run_id, state.task_id
            )));
        }
        states.insert(key, state);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::DagId;

    #[tokio::test]
    async fn test_save_and_retrieve_job() {
        let storage = InMemoryStorage::new();
        let job = StoredJob::new(JobId::new("etl_job"), "Daily ETL", DagId::new("etl_dag"));

        storage.save_job(job.clone()).await.unwrap();
        let retrieved = storage.get_job(&JobId::new("etl_job")).await.unwrap();

        assert_eq!(retrieved.id.as_str(), "etl_job");
        assert_eq!(retrieved.name, "Daily ETL");
        assert_eq!(retrieved.dag_id.as_str(), "etl_dag");
    }

    #[tokio::test]
    async fn test_list_all_jobs() {
        let storage = InMemoryStorage::new();

        // Add jobs with slight delay to ensure ordering
        storage
            .save_job(StoredJob::new(
                JobId::new("job1"),
                "Job 1",
                DagId::new("dag1"),
            ))
            .await
            .unwrap();
        storage
            .save_job(StoredJob::new(
                JobId::new("job2"),
                "Job 2",
                DagId::new("dag2"),
            ))
            .await
            .unwrap();
        storage
            .save_job(StoredJob::new(
                JobId::new("job3"),
                "Job 3",
                DagId::new("dag3"),
            ))
            .await
            .unwrap();

        let jobs = storage.list_jobs().await.unwrap();
        assert_eq!(jobs.len(), 3);
    }

    #[tokio::test]
    async fn test_delete_job() {
        let storage = InMemoryStorage::new();
        let job = StoredJob::new(JobId::new("to_delete"), "Delete Me", DagId::new("dag"));

        storage.save_job(job).await.unwrap();
        assert!(storage.get_job(&JobId::new("to_delete")).await.is_ok());

        storage.delete_job(&JobId::new("to_delete")).await.unwrap();
        assert!(storage.get_job(&JobId::new("to_delete")).await.is_err());
    }

    #[tokio::test]
    async fn test_delete_nonexistent_job_fails() {
        let storage = InMemoryStorage::new();
        let result = storage.delete_job(&JobId::new("nonexistent")).await;
        assert!(matches!(result, Err(StorageError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_save_and_retrieve_run() {
        let storage = InMemoryStorage::new();
        let run_id = RunId::new();
        let run = StoredRun::new(run_id.clone(), JobId::new("job1"));

        storage.save_run(run).await.unwrap();
        let retrieved = storage.get_run(&run_id).await.unwrap();

        assert_eq!(retrieved.job_id.as_str(), "job1");
        assert_eq!(retrieved.status, RunStatus::Pending);
    }

    #[tokio::test]
    async fn test_list_runs_for_job_with_limit() {
        let storage = InMemoryStorage::new();
        let job_id = JobId::new("job1");

        // Create 5 runs
        for _ in 0..5 {
            let run = StoredRun::new(RunId::new(), job_id.clone());
            storage.save_run(run).await.unwrap();
        }

        // Also create runs for another job
        for _ in 0..3 {
            let run = StoredRun::new(RunId::new(), JobId::new("job2"));
            storage.save_run(run).await.unwrap();
        }

        // List with limit
        let runs = storage.list_runs(&job_id, 3).await.unwrap();
        assert_eq!(runs.len(), 3);

        // All returned runs should be for job1
        for run in &runs {
            assert_eq!(run.job_id.as_str(), "job1");
        }
    }

    #[tokio::test]
    async fn test_save_and_retrieve_task_state() {
        let storage = InMemoryStorage::new();
        let run_id = RunId::new();
        let task_id = TaskId::new("extract");

        let state = StoredTaskState::new(task_id.clone(), run_id.clone());
        storage.save_task_state(state).await.unwrap();

        let retrieved = storage.get_task_state(&run_id, &task_id).await.unwrap();
        assert_eq!(retrieved.task_id.as_str(), "extract");
        assert_eq!(retrieved.status, TaskRunStatus::Pending);
    }

    #[tokio::test]
    async fn test_get_incomplete_runs() {
        let storage = InMemoryStorage::new();

        // Create runs with different statuses
        let pending = StoredRun::new(RunId::new(), JobId::new("job1"));
        storage.save_run(pending.clone()).await.unwrap();

        let mut running = StoredRun::new(RunId::new(), JobId::new("job1"));
        running.mark_running();
        storage.save_run(running.clone()).await.unwrap();

        let mut completed = StoredRun::new(RunId::new(), JobId::new("job1"));
        completed.mark_completed();
        storage.save_run(completed).await.unwrap();

        let mut failed = StoredRun::new(RunId::new(), JobId::new("job1"));
        failed.mark_failed("error");
        storage.save_run(failed).await.unwrap();

        let incomplete = storage.get_incomplete_runs().await.unwrap();
        assert_eq!(incomplete.len(), 2);

        for run in &incomplete {
            assert!(matches!(
                run.status,
                RunStatus::Pending | RunStatus::Running
            ));
        }
    }

    #[tokio::test]
    async fn test_mark_run_interrupted() {
        let storage = InMemoryStorage::new();
        let run_id = RunId::new();
        let mut run = StoredRun::new(run_id.clone(), JobId::new("job1"));
        run.mark_running();

        storage.save_run(run).await.unwrap();
        storage.mark_run_interrupted(&run_id).await.unwrap();

        let retrieved = storage.get_run(&run_id).await.unwrap();
        assert_eq!(retrieved.status, RunStatus::Interrupted);
        assert!(retrieved.ended_at.is_some());
    }

    #[tokio::test]
    async fn test_update_run_status() {
        let storage = InMemoryStorage::new();
        let run_id = RunId::new();
        let run = StoredRun::new(run_id.clone(), JobId::new("job1"));

        storage.save_run(run).await.unwrap();

        // Update to running
        let mut updated = storage.get_run(&run_id).await.unwrap();
        updated.mark_running();
        storage.update_run(updated).await.unwrap();

        let retrieved = storage.get_run(&run_id).await.unwrap();
        assert_eq!(retrieved.status, RunStatus::Running);

        // Update to completed
        let mut updated = storage.get_run(&run_id).await.unwrap();
        updated.mark_completed();
        storage.update_run(updated).await.unwrap();

        let retrieved = storage.get_run(&run_id).await.unwrap();
        assert_eq!(retrieved.status, RunStatus::Completed);
        assert!(retrieved.ended_at.is_some());
        assert!(retrieved.duration.is_some());
    }

    #[tokio::test]
    async fn test_update_task_state() {
        let storage = InMemoryStorage::new();
        let run_id = RunId::new();
        let task_id = TaskId::new("transform");

        let state = StoredTaskState::new(task_id.clone(), run_id.clone());
        storage.save_task_state(state).await.unwrap();

        // Update to running
        let mut updated = storage.get_task_state(&run_id, &task_id).await.unwrap();
        updated.mark_running();
        storage.update_task_state(updated).await.unwrap();

        let retrieved = storage.get_task_state(&run_id, &task_id).await.unwrap();
        assert_eq!(retrieved.status, TaskRunStatus::Running);
        assert_eq!(retrieved.attempts, 1);

        // Update to completed
        let mut updated = storage.get_task_state(&run_id, &task_id).await.unwrap();
        updated.mark_completed();
        storage.update_task_state(updated).await.unwrap();

        let retrieved = storage.get_task_state(&run_id, &task_id).await.unwrap();
        assert_eq!(retrieved.status, TaskRunStatus::Completed);
    }

    #[tokio::test]
    async fn test_storage_is_thread_safe() {
        use std::sync::Arc;

        let storage = Arc::new(InMemoryStorage::new());
        let mut handles = vec![];

        // Spawn multiple tasks writing concurrently
        for i in 0..10 {
            let storage = Arc::clone(&storage);
            let handle = tokio::spawn(async move {
                let job = StoredJob::new(
                    JobId::new(format!("job_{}", i)),
                    format!("Job {}", i),
                    DagId::new(format!("dag_{}", i)),
                );
                storage.save_job(job).await
            });
            handles.push(handle);
        }

        // Wait for all writes
        for handle in handles {
            handle.await.unwrap().unwrap();
        }

        // Verify all jobs were saved
        let jobs = storage.list_jobs().await.unwrap();
        assert_eq!(jobs.len(), 10);
    }

    #[tokio::test]
    async fn test_duplicate_job_fails() {
        let storage = InMemoryStorage::new();
        let job = StoredJob::new(JobId::new("dup"), "Duplicate", DagId::new("dag"));

        storage.save_job(job.clone()).await.unwrap();
        let result = storage.save_job(job).await;

        assert!(matches!(result, Err(StorageError::DuplicateKey(_))));
    }

    #[tokio::test]
    async fn test_list_task_states_for_run() {
        let storage = InMemoryStorage::new();
        let run_id = RunId::new();

        // Create multiple task states for this run
        for task_name in &["extract", "transform", "load"] {
            let state = StoredTaskState::new(TaskId::new(*task_name), run_id.clone());
            storage.save_task_state(state).await.unwrap();
        }

        // Create task states for another run
        let other_run = RunId::new();
        let state = StoredTaskState::new(TaskId::new("other"), other_run);
        storage.save_task_state(state).await.unwrap();

        let states = storage.list_task_states(&run_id).await.unwrap();
        assert_eq!(states.len(), 3);
    }

    #[tokio::test]
    async fn test_job_with_schedule() {
        let storage = InMemoryStorage::new();
        let job = StoredJob::new(JobId::new("scheduled"), "Scheduled Job", DagId::new("dag"))
            .with_schedule("0 0 * * *");

        storage.save_job(job).await.unwrap();
        let retrieved = storage.get_job(&JobId::new("scheduled")).await.unwrap();

        assert_eq!(retrieved.schedule, Some("0 0 * * *".to_string()));
    }

    #[tokio::test]
    async fn test_disabled_job() {
        let storage = InMemoryStorage::new();
        let job = StoredJob::new(JobId::new("disabled"), "Disabled Job", DagId::new("dag"))
            .with_enabled(false);

        storage.save_job(job).await.unwrap();
        let retrieved = storage.get_job(&JobId::new("disabled")).await.unwrap();

        assert!(!retrieved.enabled);
    }
}
