//! Read-only SQLite interface for the TUI.
//!
//! Provides efficient queries for displaying orchestrator state
//! without any write capabilities.

use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::core::types::{DagId, JobId, RunId, TaskId};
use crate::storage::{RunStatus, StoredJob, StoredRun, StoredTaskState, TaskRunStatus};

use super::error::TuiError;

/// Read-only database reader for the TUI.
pub struct TuiReader {
    pool: SqlitePool,
}

/// Job with aggregated run statistics.
#[derive(Debug, Clone)]
pub struct JobWithStats {
    pub job: StoredJob,
    pub total_runs: u64,
    pub successful_runs: u64,
    pub failed_runs: u64,
    pub last_run: Option<StoredRun>,
}

/// Summary statistics for the dashboard header.
#[derive(Debug, Clone, Default)]
pub struct DashboardStats {
    pub total_jobs: u64,
    pub enabled_jobs: u64,
    pub running_now: u64,
    pub pending_now: u64,
}

impl TuiReader {
    /// Open a read-only connection to the database.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, TuiError> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(TuiError::DatabaseNotFound(path.to_path_buf()));
        }

        let options = SqliteConnectOptions::from_str(&format!("sqlite:{}", path.display()))
            .map_err(|e| TuiError::Database(e.to_string()))?
            .read_only(true)
            // Wait up to 5 seconds for locks instead of failing immediately
            .busy_timeout(std::time::Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await?;

        Ok(Self { pool })
    }

    /// Get summary statistics for the status bar.
    pub async fn get_stats(&self) -> Result<DashboardStats, TuiError> {
        let job_counts: (i64, i64) = sqlx::query_as(
            "SELECT COUNT(*), SUM(CASE WHEN enabled = 1 THEN 1 ELSE 0 END) FROM jobs",
        )
        .fetch_one(&self.pool)
        .await?;

        let run_counts: (i64, i64) = sqlx::query_as(
            "SELECT
                SUM(CASE WHEN status = 'running' THEN 1 ELSE 0 END),
                SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END)
             FROM runs",
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(DashboardStats {
            total_jobs: job_counts.0 as u64,
            enabled_jobs: job_counts.1 as u64,
            running_now: run_counts.0 as u64,
            pending_now: run_counts.1 as u64,
        })
    }

    /// List all jobs with their run statistics.
    pub async fn list_jobs_with_stats(&self) -> Result<Vec<JobWithStats>, TuiError> {
        // First get all jobs
        let jobs = self.list_jobs().await?;

        let mut result = Vec::with_capacity(jobs.len());
        for job in jobs {
            let stats = self.get_job_stats(&job.id).await?;
            let last_run = self.get_last_run(&job.id).await?;
            result.push(JobWithStats {
                job,
                total_runs: stats.0,
                successful_runs: stats.1,
                failed_runs: stats.2,
                last_run,
            });
        }

        Ok(result)
    }

    /// List all jobs.
    pub async fn list_jobs(&self) -> Result<Vec<StoredJob>, TuiError> {
        #[allow(clippy::type_complexity)]
        let rows: Vec<(String, String, String, Option<String>, bool, String, String)> =
            sqlx::query_as(
                "SELECT id, name, dag_id, schedule, enabled, created_at, updated_at
                 FROM jobs ORDER BY name",
            )
            .fetch_all(&self.pool)
            .await?;

        Ok(rows.into_iter().map(row_to_job).collect())
    }

    /// Get run statistics for a job.
    async fn get_job_stats(&self, job_id: &JobId) -> Result<(u64, u64, u64), TuiError> {
        let stats: (i64, i64, i64) = sqlx::query_as(
            "SELECT
                COUNT(*),
                SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END),
                SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END)
             FROM runs WHERE job_id = ?",
        )
        .bind(job_id.as_str())
        .fetch_one(&self.pool)
        .await?;

        Ok((stats.0 as u64, stats.1 as u64, stats.2 as u64))
    }

    /// Get the most recent run for a job.
    async fn get_last_run(&self, job_id: &JobId) -> Result<Option<StoredRun>, TuiError> {
        #[allow(clippy::type_complexity)]
        let row: Option<(
            String,
            String,
            String,
            String,
            Option<String>,
            Option<i64>,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT id, job_id, status, started_at, ended_at, duration_ms, error
             FROM runs WHERE job_id = ?
             ORDER BY started_at DESC LIMIT 1",
        )
        .bind(job_id.as_str())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(row_to_run))
    }

    /// List runs for a job with pagination.
    pub async fn list_runs(
        &self,
        job_id: &JobId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<StoredRun>, TuiError> {
        #[allow(clippy::type_complexity)]
        let rows: Vec<(
            String,
            String,
            String,
            String,
            Option<String>,
            Option<i64>,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT id, job_id, status, started_at, ended_at, duration_ms, error
             FROM runs WHERE job_id = ?
             ORDER BY started_at DESC
             LIMIT ? OFFSET ?",
        )
        .bind(job_id.as_str())
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(row_to_run).collect())
    }

    /// List all recent runs across all jobs.
    pub async fn list_recent_runs(&self, limit: usize) -> Result<Vec<StoredRun>, TuiError> {
        #[allow(clippy::type_complexity)]
        let rows: Vec<(
            String,
            String,
            String,
            String,
            Option<String>,
            Option<i64>,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT id, job_id, status, started_at, ended_at, duration_ms, error
             FROM runs
             ORDER BY started_at DESC
             LIMIT ?",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(row_to_run).collect())
    }

    /// List task states for a run.
    pub async fn list_task_states(&self, run_id: &RunId) -> Result<Vec<StoredTaskState>, TuiError> {
        #[allow(clippy::type_complexity)]
        let rows: Vec<(
            String,
            String,
            String,
            i64,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<i32>,
        )> = sqlx::query_as(
            "SELECT run_id, task_id, status, attempts, started_at, ended_at, duration_ms, error, stdout, stderr, exit_code
             FROM task_states WHERE run_id = ?
             ORDER BY started_at NULLS LAST, task_id",
        )
        .bind(run_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(row_to_task_state).collect())
    }

    /// Get currently running runs.
    pub async fn get_running_runs(&self) -> Result<Vec<StoredRun>, TuiError> {
        #[allow(clippy::type_complexity)]
        let rows: Vec<(
            String,
            String,
            String,
            String,
            Option<String>,
            Option<i64>,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT id, job_id, status, started_at, ended_at, duration_ms, error
             FROM runs WHERE status = 'running'
             ORDER BY started_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(row_to_run).collect())
    }

    /// Close the connection pool.
    pub async fn close(&self) {
        self.pool.close().await;
    }
}

// Helper functions for converting database rows to domain types

fn row_to_job(row: (String, String, String, Option<String>, bool, String, String)) -> StoredJob {
    StoredJob {
        id: JobId::new(row.0),
        name: row.1,
        dag_id: DagId::new(row.2),
        schedule: row.3,
        enabled: row.4,
        created_at: string_to_system_time(&row.5),
        updated_at: string_to_system_time(&row.6),
    }
}

#[allow(clippy::type_complexity)]
fn row_to_run(
    row: (
        String,
        String,
        String,
        String,
        Option<String>,
        Option<i64>,
        Option<String>,
    ),
) -> StoredRun {
    StoredRun {
        id: RunId::from_uuid(row.0.parse().unwrap_or_default()),
        job_id: JobId::new(row.1),
        status: string_to_run_status(&row.2),
        started_at: string_to_system_time(&row.3),
        ended_at: row.4.as_ref().map(|s| string_to_system_time(s)),
        duration: row.5.map(|ms| Duration::from_millis(ms as u64)),
        error: row.6,
    }
}

#[allow(clippy::type_complexity)]
fn row_to_task_state(
    row: (
        String,
        String,
        String,
        i64,
        Option<String>,
        Option<String>,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i32>,
    ),
) -> StoredTaskState {
    StoredTaskState {
        run_id: RunId::from_uuid(row.0.parse().unwrap_or_default()),
        task_id: TaskId::new(row.1),
        status: string_to_task_status(&row.2),
        attempts: row.3 as u32,
        started_at: row.4.as_ref().map(|s| string_to_system_time(s)),
        ended_at: row.5.as_ref().map(|s| string_to_system_time(s)),
        duration: row.6.map(|ms| Duration::from_millis(ms as u64)),
        error: row.7,
        stdout: row.8,
        stderr: row.9,
        exit_code: row.10,
    }
}

fn string_to_system_time(s: &str) -> SystemTime {
    s.parse::<u128>()
        .ok()
        .map(|millis| UNIX_EPOCH + Duration::from_millis(millis as u64))
        .unwrap_or(UNIX_EPOCH)
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
