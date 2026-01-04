//! Configuration type definitions.
//!
//! This module contains the type definitions for YAML configuration structures
//! including jobs, tasks, schedules, retry policies, and dependencies.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Global configuration (petit.yaml).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GlobalConfig {
    /// Default timezone for schedules.
    pub default_timezone: Option<String>,
    /// Default retry policy for tasks.
    pub default_retry: Option<RetryConfig>,
    /// Maximum concurrent jobs.
    pub max_concurrent_jobs: Option<usize>,
    /// Maximum concurrent tasks per job.
    pub max_concurrent_tasks: Option<usize>,
    /// Storage configuration.
    pub storage: Option<StorageConfig>,
    /// Global environment variables.
    pub environment: Option<HashMap<String, String>>,
}

/// Storage configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StorageConfig {
    /// In-memory storage (default, non-persistent).
    #[serde(rename = "memory")]
    #[default]
    Memory,
    /// SQLite storage.
    #[serde(rename = "sqlite")]
    Sqlite {
        /// Path to the database file.
        path: String,
    },
}

/// Job configuration from YAML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobConfig {
    /// Job identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Schedule expression (cron or shortcut).
    pub schedule: Option<ScheduleConfig>,
    /// Task definitions.
    pub tasks: Vec<TaskConfig>,
    /// Cross-job dependencies.
    #[serde(default)]
    pub depends_on: Vec<JobDependencyConfig>,
    /// Job-level configuration values.
    #[serde(default)]
    pub config: HashMap<String, serde_yaml::Value>,
    /// Maximum concurrent runs of this job.
    pub max_concurrency: Option<usize>,
    /// Whether the job is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Schedule configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ScheduleConfig {
    /// Simple cron expression string.
    Simple(String),
    /// Detailed schedule with timezone.
    Detailed {
        /// Cron expression or shortcut.
        cron: String,
        /// Timezone for the schedule.
        timezone: Option<String>,
    },
}

impl ScheduleConfig {
    /// Get the cron expression.
    pub fn cron(&self) -> &str {
        match self {
            ScheduleConfig::Simple(s) => s,
            ScheduleConfig::Detailed { cron, .. } => cron,
        }
    }

    /// Get the timezone, if specified.
    pub fn timezone(&self) -> Option<&str> {
        match self {
            ScheduleConfig::Simple(_) => None,
            ScheduleConfig::Detailed { timezone, .. } => timezone.as_deref(),
        }
    }
}

/// Task configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskConfig {
    /// Task identifier (unique within job).
    pub id: String,
    /// Human-readable name.
    pub name: Option<String>,
    /// Task type and configuration.
    #[serde(flatten)]
    pub task_type: TaskTypeConfig,
    /// Dependencies on other tasks in this job.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Retry policy for this task.
    pub retry: Option<RetryConfig>,
    /// Environment variables for this task.
    #[serde(default)]
    pub environment: HashMap<String, String>,
    /// Execution condition.
    pub condition: Option<TaskConditionConfig>,
}

/// Task type configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TaskTypeConfig {
    /// Shell command task.
    #[serde(rename = "command")]
    Command {
        /// The command to run.
        command: String,
        /// Command arguments.
        #[serde(default)]
        args: Vec<String>,
        /// Working directory.
        working_dir: Option<String>,
        /// Timeout in seconds.
        timeout_secs: Option<u64>,
    },
    /// Python script task.
    #[serde(rename = "python")]
    Python {
        /// Python script path or inline code.
        script: String,
        /// Whether script is inline code vs file path.
        #[serde(default)]
        inline: bool,
    },
    /// Custom task type (for extensibility).
    #[serde(rename = "custom")]
    Custom {
        /// Custom task handler name.
        handler: String,
        /// Custom configuration.
        #[serde(default)]
        config: HashMap<String, serde_yaml::Value>,
    },
}

/// Task execution condition.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskConditionConfig {
    /// Run only if all upstream tasks succeeded (default).
    AllSuccess,
    /// Run only if any upstream task failed.
    OnFailure,
    /// Run regardless of upstream status.
    AllDone,
}

/// Retry policy configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Maximum number of retry attempts.
    pub max_attempts: u32,
    /// Delay between retries in seconds.
    pub delay_secs: u64,
    /// Retry condition.
    #[serde(default)]
    pub condition: RetryConditionConfig,
}

/// Retry condition configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryConditionConfig {
    /// Always retry on failure.
    #[default]
    Always,
    /// Only retry on transient errors.
    TransientOnly,
    /// Never retry.
    Never,
}

/// Cross-job dependency configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JobDependencyConfig {
    /// Simple dependency (job ID only, implies LastSuccess).
    Simple(String),
    /// Detailed dependency with condition.
    Detailed {
        /// Job ID to depend on.
        job: String,
        /// Dependency condition.
        condition: JobDependencyConditionConfig,
    },
}

impl JobDependencyConfig {
    /// Get the job ID.
    pub fn job_id(&self) -> &str {
        match self {
            JobDependencyConfig::Simple(id) => id,
            JobDependencyConfig::Detailed { job, .. } => job,
        }
    }
}

/// Job dependency condition configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobDependencyConditionConfig {
    /// Last run must have succeeded.
    LastSuccess,
    /// Must have completed (success or failure).
    LastComplete,
    /// Must have succeeded within time window.
    WithinWindow {
        /// Time window in seconds.
        seconds: u64,
    },
}
