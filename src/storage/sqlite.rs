//! SQLite storage implementation.
//!
//! Provides persistent storage using SQLite database.

use async_trait::async_trait;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::{
    RunStatus, Storage, StorageError, StoredJob, StoredRun, StoredTaskState, TaskRunStatus,
};
use crate::core::types::{DagId, JobId, RunId, TaskId};

/// SQLite storage backend.
///
/// Provides persistent storage with automatic schema migration.
pub struct SqliteStorage {
    pool: SqlitePool,
}

impl SqliteStorage {
    /// Create a new SQLite storage with the given database path.
    ///
    /// Creates the database file if it doesn't exist and runs migrations.
    pub async fn new(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path_str = path.as_ref().to_string_lossy();
        let options = SqliteConnectOptions::from_str(&format!("sqlite:{}", path_str))
            .map_err(|e| StorageError::Other(e.to_string()))?
            .create_if_missing(true)
            // WAL mode allows concurrent readers while writing
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            // Wait up to 5 seconds for locks instead of failing immediately
            .busy_timeout(Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .map_err(|e| StorageError::Other(e.to_string()))?;

        let storage = Self { pool };
        storage.run_migrations().await?;
        Ok(storage)
    }

    /// Create an in-memory SQLite database (useful for testing).
    pub async fn in_memory() -> Result<Self, StorageError> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .map_err(|e| StorageError::Other(e.to_string()))?;

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(|e| StorageError::Other(e.to_string()))?;

        let storage = Self { pool };
        storage.run_migrations().await?;
        Ok(storage)
    }

    /// Run database migrations.
    async fn run_migrations(&self) -> Result<(), StorageError> {
        let schema = include_str!("../../migrations/001_initial_schema.sql");
        sqlx::raw_sql(schema)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Other(format!("migration failed: {}", e)))?;
        Ok(())
    }

    /// Close the database connection pool.
    pub async fn close(&self) {
        self.pool.close().await;
    }
}

// Helper functions for time conversion
fn system_time_to_string(time: SystemTime) -> String {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn string_to_system_time(s: &str) -> SystemTime {
    s.parse::<u128>()
        .ok()
        .map(|millis| UNIX_EPOCH + Duration::from_millis(millis as u64))
        .unwrap_or(UNIX_EPOCH)
}

fn run_status_to_string(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Pending => "pending",
        RunStatus::Running => "running",
        RunStatus::Completed => "completed",
        RunStatus::Failed => "failed",
        RunStatus::Interrupted => "interrupted",
        RunStatus::Cancelled => "cancelled",
    }
}

fn string_to_run_status(s: &str) -> RunStatus {
    match s {
        "pending" => RunStatus::Pending,
        "running" => RunStatus::Running,
        "completed" => RunStatus::Completed,
        "failed" => RunStatus::Failed,
        "interrupted" => RunStatus::Interrupted,
        "cancelled" => RunStatus::Cancelled,
        _ => RunStatus::Pending,
    }
}

fn task_status_to_string(status: TaskRunStatus) -> &'static str {
    match status {
        TaskRunStatus::Pending => "pending",
        TaskRunStatus::Running => "running",
        TaskRunStatus::Completed => "completed",
        TaskRunStatus::Failed => "failed",
        TaskRunStatus::Skipped => "skipped",
    }
}

fn string_to_task_status(s: &str) -> TaskRunStatus {
    match s {
        "pending" => TaskRunStatus::Pending,
        "running" => TaskRunStatus::Running,
        "completed" => TaskRunStatus::Completed,
        "failed" => TaskRunStatus::Failed,
        "skipped" => TaskRunStatus::Skipped,
        _ => TaskRunStatus::Pending,
    }
}

#[async_trait]
impl Storage for SqliteStorage {
    async fn save_job(&self, job: StoredJob) -> Result<(), StorageError> {
        let result = sqlx::query(
            r#"
            INSERT INTO jobs (id, name, dag_id, schedule, enabled, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(job.id.as_str())
        .bind(&job.name)
        .bind(job.dag_id.as_str())
        .bind(&job.schedule)
        .bind(job.enabled)
        .bind(system_time_to_string(job.created_at))
        .bind(system_time_to_string(job.updated_at))
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                Err(StorageError::DuplicateKey(format!("job: {}", job.id)))
            }
            Err(e) => Err(StorageError::Other(e.to_string())),
        }
    }

    async fn upsert_job(&self, job: StoredJob) -> Result<(), StorageError> {
        sqlx::query(
            r#"
            INSERT INTO jobs (id, name, dag_id, schedule, enabled, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                dag_id = excluded.dag_id,
                schedule = excluded.schedule,
                enabled = excluded.enabled,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(job.id.as_str())
        .bind(&job.name)
        .bind(job.dag_id.as_str())
        .bind(&job.schedule)
        .bind(job.enabled)
        .bind(system_time_to_string(job.created_at))
        .bind(system_time_to_string(job.updated_at))
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Other(e.to_string()))?;
        Ok(())
    }

    async fn get_job(&self, id: &JobId) -> Result<StoredJob, StorageError> {
        let row: (String, String, String, Option<String>, bool, String, String) = sqlx::query_as(
            "SELECT id, name, dag_id, schedule, enabled, created_at, updated_at FROM jobs WHERE id = ?",
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Other(e.to_string()))?
        .ok_or_else(|| StorageError::NotFound(format!("job: {}", id)))?;

        Ok(StoredJob {
            id: JobId::new(row.0),
            name: row.1,
            dag_id: DagId::new(row.2),
            schedule: row.3,
            enabled: row.4,
            created_at: string_to_system_time(&row.5),
            updated_at: string_to_system_time(&row.6),
        })
    }

    async fn list_jobs(&self) -> Result<Vec<StoredJob>, StorageError> {
        let rows: Vec<(String, String, String, Option<String>, bool, String, String)> =
            sqlx::query_as(
                "SELECT id, name, dag_id, schedule, enabled, created_at, updated_at FROM jobs ORDER BY created_at",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StorageError::Other(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|row| StoredJob {
                id: JobId::new(row.0),
                name: row.1,
                dag_id: DagId::new(row.2),
                schedule: row.3,
                enabled: row.4,
                created_at: string_to_system_time(&row.5),
                updated_at: string_to_system_time(&row.6),
            })
            .collect())
    }

    async fn delete_job(&self, id: &JobId) -> Result<(), StorageError> {
        let result = sqlx::query("DELETE FROM jobs WHERE id = ?")
            .bind(id.as_str())
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Other(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound(format!("job: {}", id)));
        }
        Ok(())
    }

    async fn save_run(&self, run: StoredRun) -> Result<(), StorageError> {
        let result = sqlx::query(
            r#"
            INSERT INTO runs (id, job_id, status, started_at, ended_at, duration_ms, error)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(run.id.to_string())
        .bind(run.job_id.as_str())
        .bind(run_status_to_string(run.status))
        .bind(system_time_to_string(run.started_at))
        .bind(run.ended_at.map(system_time_to_string))
        .bind(run.duration.map(|d| d.as_millis() as i64))
        .bind(&run.error)
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                Err(StorageError::DuplicateKey(format!("run: {}", run.id)))
            }
            Err(e) => Err(StorageError::Other(e.to_string())),
        }
    }

    async fn get_run(&self, id: &RunId) -> Result<StoredRun, StorageError> {
        let row: (
            String,
            String,
            String,
            String,
            Option<String>,
            Option<i64>,
            Option<String>,
        ) = sqlx::query_as(
            "SELECT id, job_id, status, started_at, ended_at, duration_ms, error FROM runs WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Other(e.to_string()))?
        .ok_or_else(|| StorageError::NotFound(format!("run: {}", id)))?;

        Ok(StoredRun {
            id: RunId::from_uuid(
                row.0
                    .parse()
                    .map_err(|e| StorageError::Other(format!("invalid uuid: {}", e)))?,
            ),
            job_id: JobId::new(row.1),
            status: string_to_run_status(&row.2),
            started_at: string_to_system_time(&row.3),
            ended_at: row.4.as_ref().map(|s| string_to_system_time(s)),
            duration: row.5.map(|ms| Duration::from_millis(ms as u64)),
            error: row.6,
        })
    }

    async fn list_runs(
        &self,
        job_id: &JobId,
        limit: usize,
    ) -> Result<Vec<StoredRun>, StorageError> {
        let rows: Vec<(
            String,
            String,
            String,
            String,
            Option<String>,
            Option<i64>,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT id, job_id, status, started_at, ended_at, duration_ms, error FROM runs WHERE job_id = ? ORDER BY started_at DESC LIMIT ?",
        )
        .bind(job_id.as_str())
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Other(e.to_string()))?;

        rows.into_iter()
            .map(|row| {
                Ok(StoredRun {
                    id: RunId::from_uuid(
                        row.0
                            .parse()
                            .map_err(|e| StorageError::Other(format!("invalid uuid: {}", e)))?,
                    ),
                    job_id: JobId::new(row.1),
                    status: string_to_run_status(&row.2),
                    started_at: string_to_system_time(&row.3),
                    ended_at: row.4.as_ref().map(|s| string_to_system_time(s)),
                    duration: row.5.map(|ms| Duration::from_millis(ms as u64)),
                    error: row.6,
                })
            })
            .collect()
    }

    async fn update_run(&self, run: StoredRun) -> Result<(), StorageError> {
        let result = sqlx::query(
            r#"
            UPDATE runs SET status = ?, started_at = ?, ended_at = ?, duration_ms = ?, error = ?
            WHERE id = ?
            "#,
        )
        .bind(run_status_to_string(run.status))
        .bind(system_time_to_string(run.started_at))
        .bind(run.ended_at.map(system_time_to_string))
        .bind(run.duration.map(|d| d.as_millis() as i64))
        .bind(&run.error)
        .bind(run.id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Other(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound(format!("run: {}", run.id)));
        }
        Ok(())
    }

    async fn get_incomplete_runs(&self) -> Result<Vec<StoredRun>, StorageError> {
        let rows: Vec<(
            String,
            String,
            String,
            String,
            Option<String>,
            Option<i64>,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT id, job_id, status, started_at, ended_at, duration_ms, error FROM runs WHERE status IN ('pending', 'running')",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Other(e.to_string()))?;

        rows.into_iter()
            .map(|row| {
                Ok(StoredRun {
                    id: RunId::from_uuid(
                        row.0
                            .parse()
                            .map_err(|e| StorageError::Other(format!("invalid uuid: {}", e)))?,
                    ),
                    job_id: JobId::new(row.1),
                    status: string_to_run_status(&row.2),
                    started_at: string_to_system_time(&row.3),
                    ended_at: row.4.as_ref().map(|s| string_to_system_time(s)),
                    duration: row.5.map(|ms| Duration::from_millis(ms as u64)),
                    error: row.6,
                })
            })
            .collect()
    }

    async fn mark_run_interrupted(&self, id: &RunId) -> Result<(), StorageError> {
        let now = system_time_to_string(SystemTime::now());
        let result =
            sqlx::query("UPDATE runs SET status = 'interrupted', ended_at = ? WHERE id = ?")
                .bind(&now)
                .bind(id.to_string())
                .execute(&self.pool)
                .await
                .map_err(|e| StorageError::Other(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound(format!("run: {}", id)));
        }
        Ok(())
    }

    async fn save_task_state(&self, state: StoredTaskState) -> Result<(), StorageError> {
        let result = sqlx::query(
            r#"
            INSERT INTO task_states (run_id, task_id, status, attempts, started_at, ended_at, duration_ms, error)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(state.run_id.to_string())
        .bind(state.task_id.as_str())
        .bind(task_status_to_string(state.status))
        .bind(state.attempts as i64)
        .bind(state.started_at.map(system_time_to_string))
        .bind(state.ended_at.map(system_time_to_string))
        .bind(state.duration.map(|d| d.as_millis() as i64))
        .bind(&state.error)
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                Err(StorageError::DuplicateKey(format!(
                    "task_state: {}/{}",
                    state.run_id, state.task_id
                )))
            }
            Err(e) => Err(StorageError::Other(e.to_string())),
        }
    }

    async fn get_task_state(
        &self,
        run_id: &RunId,
        task_id: &TaskId,
    ) -> Result<StoredTaskState, StorageError> {
        let row: (
            String,
            String,
            String,
            i64,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<String>,
        ) = sqlx::query_as(
            "SELECT run_id, task_id, status, attempts, started_at, ended_at, duration_ms, error FROM task_states WHERE run_id = ? AND task_id = ?",
        )
        .bind(run_id.to_string())
        .bind(task_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Other(e.to_string()))?
        .ok_or_else(|| StorageError::NotFound(format!("task_state: {}/{}", run_id, task_id)))?;

        Ok(StoredTaskState {
            run_id: RunId::from_uuid(
                row.0
                    .parse()
                    .map_err(|e| StorageError::Other(format!("invalid uuid: {}", e)))?,
            ),
            task_id: TaskId::new(row.1),
            status: string_to_task_status(&row.2),
            attempts: row.3 as u32,
            started_at: row.4.as_ref().map(|s| string_to_system_time(s)),
            ended_at: row.5.as_ref().map(|s| string_to_system_time(s)),
            duration: row.6.map(|ms| Duration::from_millis(ms as u64)),
            error: row.7,
        })
    }

    async fn list_task_states(&self, run_id: &RunId) -> Result<Vec<StoredTaskState>, StorageError> {
        let rows: Vec<(
            String,
            String,
            String,
            i64,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT run_id, task_id, status, attempts, started_at, ended_at, duration_ms, error FROM task_states WHERE run_id = ?",
        )
        .bind(run_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Other(e.to_string()))?;

        rows.into_iter()
            .map(|row| {
                Ok(StoredTaskState {
                    run_id: RunId::from_uuid(
                        row.0
                            .parse()
                            .map_err(|e| StorageError::Other(format!("invalid uuid: {}", e)))?,
                    ),
                    task_id: TaskId::new(row.1),
                    status: string_to_task_status(&row.2),
                    attempts: row.3 as u32,
                    started_at: row.4.as_ref().map(|s| string_to_system_time(s)),
                    ended_at: row.5.as_ref().map(|s| string_to_system_time(s)),
                    duration: row.6.map(|ms| Duration::from_millis(ms as u64)),
                    error: row.7,
                })
            })
            .collect()
    }

    async fn update_task_state(&self, state: StoredTaskState) -> Result<(), StorageError> {
        let result = sqlx::query(
            r#"
            UPDATE task_states SET status = ?, attempts = ?, started_at = ?, ended_at = ?, duration_ms = ?, error = ?
            WHERE run_id = ? AND task_id = ?
            "#,
        )
        .bind(task_status_to_string(state.status))
        .bind(state.attempts as i64)
        .bind(state.started_at.map(system_time_to_string))
        .bind(state.ended_at.map(system_time_to_string))
        .bind(state.duration.map(|d| d.as_millis() as i64))
        .bind(&state.error)
        .bind(state.run_id.to_string())
        .bind(state.task_id.as_str())
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Other(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound(format!(
                "task_state: {}/{}",
                state.run_id, state.task_id
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn create_test_storage() -> SqliteStorage {
        SqliteStorage::in_memory().await.unwrap()
    }

    #[tokio::test]
    async fn test_initialize_database_schema() {
        let storage = create_test_storage().await;
        // If we got here without error, schema was initialized
        storage.close().await;
    }

    #[tokio::test]
    async fn test_save_and_retrieve_job() {
        let storage = create_test_storage().await;
        let job = StoredJob::new(JobId::new("etl_job"), "Daily ETL", DagId::new("etl_dag"));

        storage.save_job(job.clone()).await.unwrap();
        let retrieved = storage.get_job(&JobId::new("etl_job")).await.unwrap();

        assert_eq!(retrieved.id.as_str(), "etl_job");
        assert_eq!(retrieved.name, "Daily ETL");
        assert_eq!(retrieved.dag_id.as_str(), "etl_dag");
        storage.close().await;
    }

    #[tokio::test]
    async fn test_job_persists_across_connection() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");

        // Create and save job
        {
            let storage = SqliteStorage::new(&db_path).await.unwrap();
            let job = StoredJob::new(
                JobId::new("persist_test"),
                "Persist Test",
                DagId::new("dag"),
            );
            storage.save_job(job).await.unwrap();
            storage.close().await;
        }

        // Reconnect and verify
        {
            let storage = SqliteStorage::new(&db_path).await.unwrap();
            let retrieved = storage.get_job(&JobId::new("persist_test")).await.unwrap();
            assert_eq!(retrieved.id.as_str(), "persist_test");
            storage.close().await;
        }
    }

    #[tokio::test]
    async fn test_list_all_jobs() {
        let storage = create_test_storage().await;

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
        storage.close().await;
    }

    #[tokio::test]
    async fn test_delete_job() {
        let storage = create_test_storage().await;
        let job = StoredJob::new(JobId::new("to_delete"), "Delete Me", DagId::new("dag"));

        storage.save_job(job).await.unwrap();
        assert!(storage.get_job(&JobId::new("to_delete")).await.is_ok());

        storage.delete_job(&JobId::new("to_delete")).await.unwrap();
        assert!(storage.get_job(&JobId::new("to_delete")).await.is_err());
        storage.close().await;
    }

    #[tokio::test]
    async fn test_save_and_retrieve_run() {
        let storage = create_test_storage().await;

        // First create a job
        let job = StoredJob::new(JobId::new("job1"), "Job 1", DagId::new("dag1"));
        storage.save_job(job).await.unwrap();

        let run_id = RunId::new();
        let run = StoredRun::new(run_id.clone(), JobId::new("job1"));

        storage.save_run(run).await.unwrap();
        let retrieved = storage.get_run(&run_id).await.unwrap();

        assert_eq!(retrieved.job_id.as_str(), "job1");
        assert_eq!(retrieved.status, RunStatus::Pending);
        storage.close().await;
    }

    #[tokio::test]
    async fn test_list_runs_for_job_with_limit() {
        let storage = create_test_storage().await;

        let job = StoredJob::new(JobId::new("job1"), "Job 1", DagId::new("dag1"));
        storage.save_job(job).await.unwrap();

        // Create 5 runs
        for _ in 0..5 {
            let run = StoredRun::new(RunId::new(), JobId::new("job1"));
            storage.save_run(run).await.unwrap();
        }

        // List with limit
        let runs = storage.list_runs(&JobId::new("job1"), 3).await.unwrap();
        assert_eq!(runs.len(), 3);
        storage.close().await;
    }

    #[tokio::test]
    async fn test_save_and_retrieve_task_state() {
        let storage = create_test_storage().await;

        // Create job and run first
        let job = StoredJob::new(JobId::new("job1"), "Job 1", DagId::new("dag1"));
        storage.save_job(job).await.unwrap();

        let run_id = RunId::new();
        let run = StoredRun::new(run_id.clone(), JobId::new("job1"));
        storage.save_run(run).await.unwrap();

        let task_id = TaskId::new("extract");
        let state = StoredTaskState::new(task_id.clone(), run_id.clone());
        storage.save_task_state(state).await.unwrap();

        let retrieved = storage.get_task_state(&run_id, &task_id).await.unwrap();
        assert_eq!(retrieved.task_id.as_str(), "extract");
        assert_eq!(retrieved.status, TaskRunStatus::Pending);
        storage.close().await;
    }

    #[tokio::test]
    async fn test_get_incomplete_runs() {
        let storage = create_test_storage().await;

        let job = StoredJob::new(JobId::new("job1"), "Job 1", DagId::new("dag1"));
        storage.save_job(job).await.unwrap();

        // Create runs with different statuses
        let pending = StoredRun::new(RunId::new(), JobId::new("job1"));
        storage.save_run(pending).await.unwrap();

        let mut running = StoredRun::new(RunId::new(), JobId::new("job1"));
        running.mark_running();
        storage.save_run(running).await.unwrap();

        let mut completed = StoredRun::new(RunId::new(), JobId::new("job1"));
        completed.mark_completed();
        storage.save_run(completed).await.unwrap();

        let incomplete = storage.get_incomplete_runs().await.unwrap();
        assert_eq!(incomplete.len(), 2);
        storage.close().await;
    }

    #[tokio::test]
    async fn test_mark_run_interrupted() {
        let storage = create_test_storage().await;

        let job = StoredJob::new(JobId::new("job1"), "Job 1", DagId::new("dag1"));
        storage.save_job(job).await.unwrap();

        let run_id = RunId::new();
        let mut run = StoredRun::new(run_id.clone(), JobId::new("job1"));
        run.mark_running();
        storage.save_run(run).await.unwrap();

        storage.mark_run_interrupted(&run_id).await.unwrap();

        let retrieved = storage.get_run(&run_id).await.unwrap();
        assert_eq!(retrieved.status, RunStatus::Interrupted);
        assert!(retrieved.ended_at.is_some());
        storage.close().await;
    }

    #[tokio::test]
    async fn test_update_run_status() {
        let storage = create_test_storage().await;

        let job = StoredJob::new(JobId::new("job1"), "Job 1", DagId::new("dag1"));
        storage.save_job(job).await.unwrap();

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
        storage.close().await;
    }

    #[tokio::test]
    async fn test_duplicate_job_fails() {
        let storage = create_test_storage().await;
        let job = StoredJob::new(JobId::new("dup"), "Duplicate", DagId::new("dag"));

        storage.save_job(job.clone()).await.unwrap();
        let result = storage.save_job(job).await;

        assert!(matches!(result, Err(StorageError::DuplicateKey(_))));
        storage.close().await;
    }

    #[tokio::test]
    async fn test_concurrent_writes() {
        use std::sync::Arc;

        let storage = Arc::new(create_test_storage().await);
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
        storage.close().await;
    }

    #[tokio::test]
    async fn test_migration_is_idempotent() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");

        // Create storage (runs migrations)
        {
            let storage = SqliteStorage::new(&db_path).await.unwrap();
            let job = StoredJob::new(JobId::new("test"), "Test", DagId::new("dag"));
            storage.save_job(job).await.unwrap();
            storage.close().await;
        }

        // Create storage again (runs migrations again - should be idempotent)
        {
            let storage = SqliteStorage::new(&db_path).await.unwrap();
            // Should still have our job
            let jobs = storage.list_jobs().await.unwrap();
            assert_eq!(jobs.len(), 1);
            storage.close().await;
        }
    }

    #[tokio::test]
    async fn test_list_task_states_for_run() {
        let storage = create_test_storage().await;

        let job = StoredJob::new(JobId::new("job1"), "Job 1", DagId::new("dag1"));
        storage.save_job(job).await.unwrap();

        let run_id = RunId::new();
        let run = StoredRun::new(run_id.clone(), JobId::new("job1"));
        storage.save_run(run).await.unwrap();

        // Create multiple task states
        for task_name in &["extract", "transform", "load"] {
            let state = StoredTaskState::new(TaskId::new(*task_name), run_id.clone());
            storage.save_task_state(state).await.unwrap();
        }

        let states = storage.list_task_states(&run_id).await.unwrap();
        assert_eq!(states.len(), 3);
        storage.close().await;
    }

    #[tokio::test]
    async fn test_update_task_state() {
        let storage = create_test_storage().await;

        let job = StoredJob::new(JobId::new("job1"), "Job 1", DagId::new("dag1"));
        storage.save_job(job).await.unwrap();

        let run_id = RunId::new();
        let run = StoredRun::new(run_id.clone(), JobId::new("job1"));
        storage.save_run(run).await.unwrap();

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
        storage.close().await;
    }
}
