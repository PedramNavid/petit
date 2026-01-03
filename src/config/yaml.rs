//! YAML configuration parsing.
//!
//! Parses job definitions and global configuration from YAML files.

use crate::core::dag::TaskCondition;
use crate::core::job::DependencyCondition;
use crate::core::retry::{RetryCondition, RetryPolicy};
use crate::core::schedule::Schedule;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use thiserror::Error;

/// Errors that can occur when loading configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Failed to read configuration file.
    #[error("failed to read file: {0}")]
    IoError(#[from] std::io::Error),

    /// Failed to parse YAML.
    #[error("YAML parse error: {0}")]
    YamlError(#[from] serde_yaml::Error),

    /// Invalid configuration value.
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    /// Missing required field.
    #[error("missing required field: {0}")]
    MissingField(String),
}

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

/// YAML configuration loader.
pub struct YamlLoader;

impl YamlLoader {
    /// Load global configuration from a file.
    pub fn load_global_config(path: impl AsRef<Path>) -> Result<GlobalConfig, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        Self::parse_global_config(&content)
    }

    /// Parse global configuration from a YAML string.
    pub fn parse_global_config(yaml: &str) -> Result<GlobalConfig, ConfigError> {
        let config: GlobalConfig = serde_yaml::from_str(yaml)?;
        Ok(config)
    }

    /// Load a job configuration from a file.
    pub fn load_job_config(path: impl AsRef<Path>) -> Result<JobConfig, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        Self::parse_job_config(&content)
    }

    /// Parse a job configuration from a YAML string.
    pub fn parse_job_config(yaml: &str) -> Result<JobConfig, ConfigError> {
        let config: JobConfig = serde_yaml::from_str(yaml)?;
        Self::validate_job_config(&config)?;
        Ok(config)
    }

    /// Validate a job configuration.
    fn validate_job_config(config: &JobConfig) -> Result<(), ConfigError> {
        // Check for empty ID
        if config.id.is_empty() {
            return Err(ConfigError::MissingField("id".into()));
        }

        // Check for empty name
        if config.name.is_empty() {
            return Err(ConfigError::MissingField("name".into()));
        }

        // Check for at least one task
        if config.tasks.is_empty() {
            return Err(ConfigError::InvalidConfig(
                "job must have at least one task".into(),
            ));
        }

        // Check that max_concurrency is not zero (would make jobs un-runnable)
        if config.max_concurrency == Some(0) {
            return Err(ConfigError::InvalidConfig(
                "max_concurrency cannot be zero".into(),
            ));
        }

        // Check for duplicate task IDs
        let mut task_ids: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for task in &config.tasks {
            if !task_ids.insert(&task.id) {
                return Err(ConfigError::InvalidConfig(format!(
                    "duplicate task id: {}",
                    task.id
                )));
            }
        }

        // Check that task dependencies reference valid tasks
        for task in &config.tasks {
            for dep in &task.depends_on {
                if !task_ids.contains(dep.as_str()) {
                    return Err(ConfigError::InvalidConfig(format!(
                        "task '{}' depends on unknown task '{}'",
                        task.id, dep
                    )));
                }
            }
        }

        Ok(())
    }

    /// Convert retry config to Duration.
    pub fn retry_delay(config: &RetryConfig) -> Duration {
        Duration::from_secs(config.delay_secs)
    }
}

// ============================================================================
// Conversion implementations from config types to core types
// ============================================================================

impl TryFrom<RetryConfig> for RetryPolicy {
    type Error = ConfigError;

    fn try_from(config: RetryConfig) -> Result<Self, Self::Error> {
        let condition = match config.condition {
            RetryConditionConfig::Always => RetryCondition::Always,
            RetryConditionConfig::TransientOnly => RetryCondition::TransientOnly,
            RetryConditionConfig::Never => RetryCondition::Never,
        };

        Ok(
            RetryPolicy::fixed(config.max_attempts, Duration::from_secs(config.delay_secs))
                .with_condition(condition),
        )
    }
}

impl TryFrom<ScheduleConfig> for Schedule {
    type Error = ConfigError;

    fn try_from(config: ScheduleConfig) -> Result<Self, Self::Error> {
        match config {
            ScheduleConfig::Simple(cron) => {
                Schedule::new(cron).map_err(|e| ConfigError::InvalidConfig(e.to_string()))
            }
            ScheduleConfig::Detailed { cron, timezone } => {
                if let Some(tz) = timezone {
                    Schedule::with_timezone(cron, tz)
                        .map_err(|e| ConfigError::InvalidConfig(e.to_string()))
                } else {
                    Schedule::new(cron).map_err(|e| ConfigError::InvalidConfig(e.to_string()))
                }
            }
        }
    }
}

impl From<TaskConditionConfig> for TaskCondition {
    fn from(config: TaskConditionConfig) -> Self {
        match config {
            TaskConditionConfig::AllSuccess => TaskCondition::AllSuccess,
            TaskConditionConfig::OnFailure => TaskCondition::OnFailure,
            TaskConditionConfig::AllDone => TaskCondition::AllDone,
        }
    }
}

impl TryFrom<JobDependencyConditionConfig> for DependencyCondition {
    type Error = ConfigError;

    fn try_from(config: JobDependencyConditionConfig) -> Result<Self, Self::Error> {
        match config {
            JobDependencyConditionConfig::LastSuccess => Ok(DependencyCondition::LastSuccess),
            JobDependencyConditionConfig::LastComplete => Ok(DependencyCondition::LastComplete),
            JobDependencyConditionConfig::WithinWindow { seconds } => Ok(
                DependencyCondition::WithinWindow(Duration::from_secs(seconds)),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_job_yaml() {
        let yaml = r#"
id: minimal_job
name: Minimal Job
tasks:
  - id: task1
    type: command
    command: echo
    args: ["hello"]
"#;
        let config = YamlLoader::parse_job_config(yaml).unwrap();
        assert_eq!(config.id, "minimal_job");
        assert_eq!(config.name, "Minimal Job");
        assert_eq!(config.tasks.len(), 1);
        assert!(config.enabled);
    }

    #[test]
    fn test_parse_job_with_all_fields() {
        let yaml = r#"
id: full_job
name: Full Job
description: A job with all fields specified
schedule:
  cron: "0 9 * * *"
  timezone: America/New_York
tasks:
  - id: extract
    name: Extract Data
    type: command
    command: ./extract.sh
    environment:
      DB_HOST: localhost
    retry:
      max_attempts: 3
      delay_secs: 60
  - id: transform
    type: command
    command: ./transform.sh
    depends_on: [extract]
    condition: all_success
  - id: load
    type: command
    command: ./load.sh
    depends_on: [transform]
depends_on:
  - upstream_job
config:
  batch_size: 1000
  debug: true
max_concurrency: 1
enabled: true
"#;
        let config = YamlLoader::parse_job_config(yaml).unwrap();
        assert_eq!(config.id, "full_job");
        assert_eq!(
            config.description,
            Some("A job with all fields specified".to_string())
        );
        assert!(config.schedule.is_some());
        assert_eq!(config.tasks.len(), 3);
        assert_eq!(config.depends_on.len(), 1);
        assert_eq!(config.max_concurrency, Some(1));
    }

    #[test]
    fn test_parse_task_with_command_type() {
        let yaml = r#"
id: cmd_job
name: Command Job
tasks:
  - id: run_script
    type: command
    command: /usr/bin/python
    args: ["-c", "print('hello')"]
    working_dir: /tmp
    timeout_secs: 300
"#;
        let config = YamlLoader::parse_job_config(yaml).unwrap();
        let task = &config.tasks[0];

        match &task.task_type {
            TaskTypeConfig::Command {
                command,
                args,
                working_dir,
                timeout_secs,
            } => {
                assert_eq!(command, "/usr/bin/python");
                assert_eq!(args, &vec!["-c", "print('hello')"]);
                assert_eq!(working_dir, &Some("/tmp".to_string()));
                assert_eq!(timeout_secs, &Some(300));
            }
            _ => panic!("Expected Command task type"),
        }
    }

    #[test]
    fn test_parse_task_dependencies() {
        let yaml = r#"
id: dep_job
name: Dependency Job
tasks:
  - id: first
    type: command
    command: echo
    args: ["first"]
  - id: second
    type: command
    command: echo
    args: ["second"]
    depends_on: [first]
  - id: third
    type: command
    command: echo
    args: ["third"]
    depends_on: [first, second]
"#;
        let config = YamlLoader::parse_job_config(yaml).unwrap();
        assert_eq!(config.tasks[0].depends_on.len(), 0);
        assert_eq!(config.tasks[1].depends_on, vec!["first"]);
        assert_eq!(config.tasks[2].depends_on, vec!["first", "second"]);
    }

    #[test]
    fn test_parse_retry_policy() {
        let yaml = r#"
id: retry_job
name: Retry Job
tasks:
  - id: flaky_task
    type: command
    command: ./flaky.sh
    retry:
      max_attempts: 5
      delay_secs: 30
      condition: transient_only
"#;
        let config = YamlLoader::parse_job_config(yaml).unwrap();
        let retry = config.tasks[0].retry.as_ref().unwrap();
        assert_eq!(retry.max_attempts, 5);
        assert_eq!(retry.delay_secs, 30);
        assert!(matches!(
            retry.condition,
            RetryConditionConfig::TransientOnly
        ));
    }

    #[test]
    fn test_parse_schedule_with_timezone() {
        let yaml = r#"
id: scheduled_job
name: Scheduled Job
schedule:
  cron: "0 9 * * 1-5"
  timezone: Europe/London
tasks:
  - id: task1
    type: command
    command: echo
    args: ["scheduled"]
"#;
        let config = YamlLoader::parse_job_config(yaml).unwrap();
        let schedule = config.schedule.unwrap();
        assert_eq!(schedule.cron(), "0 9 * * 1-5");
        assert_eq!(schedule.timezone(), Some("Europe/London"));
    }

    #[test]
    fn test_parse_simple_schedule() {
        let yaml = r#"
id: simple_schedule_job
name: Simple Schedule Job
schedule: "@daily"
tasks:
  - id: task1
    type: command
    command: echo
    args: ["daily"]
"#;
        let config = YamlLoader::parse_job_config(yaml).unwrap();
        let schedule = config.schedule.unwrap();
        assert_eq!(schedule.cron(), "@daily");
        assert_eq!(schedule.timezone(), None);
    }

    #[test]
    fn test_parse_cross_job_dependencies() {
        let yaml = r#"
id: downstream_job
name: Downstream Job
depends_on:
  - upstream_job_1
  - job: upstream_job_2
    condition: last_complete
  - job: upstream_job_3
    condition:
      within_window:
        seconds: 3600
tasks:
  - id: task1
    type: command
    command: echo
    args: ["downstream"]
"#;
        let config = YamlLoader::parse_job_config(yaml).unwrap();
        assert_eq!(config.depends_on.len(), 3);

        // Simple dependency
        assert_eq!(config.depends_on[0].job_id(), "upstream_job_1");

        // Detailed dependency with last_complete
        match &config.depends_on[1] {
            JobDependencyConfig::Detailed { condition, .. } => {
                assert!(matches!(
                    condition,
                    JobDependencyConditionConfig::LastComplete
                ));
            }
            _ => panic!("Expected detailed dependency"),
        }

        // Detailed dependency with within_window
        match &config.depends_on[2] {
            JobDependencyConfig::Detailed { condition, .. } => match condition {
                JobDependencyConditionConfig::WithinWindow { seconds } => {
                    assert_eq!(*seconds, 3600);
                }
                _ => panic!("Expected WithinWindow condition"),
            },
            _ => panic!("Expected detailed dependency"),
        }
    }

    #[test]
    fn test_validation_error_missing_id() {
        let yaml = r#"
id: ""
name: No ID Job
tasks:
  - id: task1
    type: command
    command: echo
"#;
        let result = YamlLoader::parse_job_config(yaml);
        assert!(matches!(result, Err(ConfigError::MissingField(_))));
    }

    #[test]
    fn test_validation_error_no_tasks() {
        let yaml = r#"
id: no_tasks_job
name: No Tasks Job
tasks: []
"#;
        let result = YamlLoader::parse_job_config(yaml);
        assert!(matches!(result, Err(ConfigError::InvalidConfig(_))));
    }

    #[test]
    fn test_validation_error_duplicate_task_id() {
        let yaml = r#"
id: dup_task_job
name: Duplicate Task Job
tasks:
  - id: task1
    type: command
    command: echo
  - id: task1
    type: command
    command: echo
"#;
        let result = YamlLoader::parse_job_config(yaml);
        assert!(matches!(result, Err(ConfigError::InvalidConfig(_))));
    }

    #[test]
    fn test_validation_error_invalid_dependency() {
        let yaml = r#"
id: invalid_dep_job
name: Invalid Dependency Job
tasks:
  - id: task1
    type: command
    command: echo
    depends_on: [nonexistent]
"#;
        let result = YamlLoader::parse_job_config(yaml);
        assert!(matches!(result, Err(ConfigError::InvalidConfig(_))));
    }

    #[test]
    fn test_validation_error_zero_max_concurrency() {
        let yaml = r#"
id: zero_concurrency_job
name: Zero Concurrency Job
max_concurrency: 0
tasks:
  - id: task1
    type: command
    command: echo
"#;
        let result = YamlLoader::parse_job_config(yaml);
        assert!(matches!(result, Err(ConfigError::InvalidConfig(_))));
        if let Err(ConfigError::InvalidConfig(msg)) = result {
            assert!(msg.contains("max_concurrency cannot be zero"));
        }
    }

    #[test]
    fn test_parse_global_config() {
        let yaml = r#"
default_timezone: UTC
default_retry:
  max_attempts: 3
  delay_secs: 60
max_concurrent_jobs: 10
max_concurrent_tasks: 5
storage:
  type: sqlite
  path: /var/lib/petit/petit.db
environment:
  LOG_LEVEL: info
"#;
        let config = YamlLoader::parse_global_config(yaml).unwrap();
        assert_eq!(config.default_timezone, Some("UTC".to_string()));
        assert_eq!(config.max_concurrent_jobs, Some(10));
        assert_eq!(config.max_concurrent_tasks, Some(5));

        match config.storage {
            Some(StorageConfig::Sqlite { path }) => {
                assert_eq!(path, "/var/lib/petit/petit.db");
            }
            _ => panic!("Expected SQLite storage config"),
        }
    }

    #[test]
    fn test_parse_empty_global_config() {
        let yaml = "{}";
        let config = YamlLoader::parse_global_config(yaml).unwrap();
        assert!(config.default_timezone.is_none());
        assert!(config.default_retry.is_none());
    }

    #[test]
    fn test_parse_python_task() {
        let yaml = r#"
id: python_job
name: Python Job
tasks:
  - id: run_python
    type: python
    script: print("hello from python")
    inline: true
"#;
        let config = YamlLoader::parse_job_config(yaml).unwrap();
        match &config.tasks[0].task_type {
            TaskTypeConfig::Python { script, inline } => {
                assert_eq!(script, "print(\"hello from python\")");
                assert!(*inline);
            }
            _ => panic!("Expected Python task type"),
        }
    }

    #[test]
    fn test_parse_task_conditions() {
        let yaml = r#"
id: condition_job
name: Condition Job
tasks:
  - id: main_task
    type: command
    command: ./main.sh
  - id: cleanup
    type: command
    command: ./cleanup.sh
    depends_on: [main_task]
    condition: all_done
  - id: on_error
    type: command
    command: ./notify_error.sh
    depends_on: [main_task]
    condition: on_failure
"#;
        let config = YamlLoader::parse_job_config(yaml).unwrap();
        assert!(config.tasks[0].condition.is_none());
        assert!(matches!(
            config.tasks[1].condition,
            Some(TaskConditionConfig::AllDone)
        ));
        assert!(matches!(
            config.tasks[2].condition,
            Some(TaskConditionConfig::OnFailure)
        ));
    }

    #[test]
    fn test_parse_task_environment() {
        let yaml = r#"
id: env_job
name: Environment Job
tasks:
  - id: task_with_env
    type: command
    command: ./run.sh
    environment:
      DATABASE_URL: postgres://localhost/db
      API_KEY: secret123
      DEBUG: "true"
"#;
        let config = YamlLoader::parse_job_config(yaml).unwrap();
        let env = &config.tasks[0].environment;
        assert_eq!(
            env.get("DATABASE_URL"),
            Some(&"postgres://localhost/db".to_string())
        );
        assert_eq!(env.get("API_KEY"), Some(&"secret123".to_string()));
        assert_eq!(env.get("DEBUG"), Some(&"true".to_string()));
    }

    #[test]
    fn test_parse_custom_task_type() {
        let yaml = r#"
id: custom_job
name: Custom Job
tasks:
  - id: custom_task
    type: custom
    handler: my_custom_handler
    config:
      param1: value1
      param2: 42
"#;
        let config = YamlLoader::parse_job_config(yaml).unwrap();
        match &config.tasks[0].task_type {
            TaskTypeConfig::Custom { handler, config } => {
                assert_eq!(handler, "my_custom_handler");
                assert!(config.contains_key("param1"));
                assert!(config.contains_key("param2"));
            }
            _ => panic!("Expected Custom task type"),
        }
    }

    // ========================================================================
    // Conversion tests
    // ========================================================================

    #[test]
    fn test_retry_config_to_retry_policy_with_always_condition() {
        let config = RetryConfig {
            max_attempts: 3,
            delay_secs: 60,
            condition: RetryConditionConfig::Always,
        };

        let policy: RetryPolicy = config.try_into().unwrap();

        assert_eq!(policy.max_attempts, 3);
        assert_eq!(policy.delay, Duration::from_secs(60));
        assert_eq!(policy.retry_on, RetryCondition::Always);
    }

    #[test]
    fn test_retry_config_to_retry_policy_with_transient_only() {
        let config = RetryConfig {
            max_attempts: 5,
            delay_secs: 30,
            condition: RetryConditionConfig::TransientOnly,
        };

        let policy: RetryPolicy = config.try_into().unwrap();

        assert_eq!(policy.max_attempts, 5);
        assert_eq!(policy.delay, Duration::from_secs(30));
        assert_eq!(policy.retry_on, RetryCondition::TransientOnly);
    }

    #[test]
    fn test_retry_config_to_retry_policy_with_never() {
        let config = RetryConfig {
            max_attempts: 0,
            delay_secs: 0,
            condition: RetryConditionConfig::Never,
        };

        let policy: RetryPolicy = config.try_into().unwrap();

        assert_eq!(policy.max_attempts, 0);
        assert_eq!(policy.delay, Duration::from_secs(0));
        assert_eq!(policy.retry_on, RetryCondition::Never);
        assert!(!policy.is_enabled());
    }

    #[test]
    fn test_schedule_config_simple_to_schedule() {
        let config = ScheduleConfig::Simple("@daily".to_string());

        let schedule: Schedule = config.try_into().unwrap();

        assert_eq!(schedule.expression(), "@daily");
        assert_eq!(schedule.timezone(), "UTC");
    }

    #[test]
    fn test_schedule_config_simple_with_cron_to_schedule() {
        let config = ScheduleConfig::Simple("0 9 * * *".to_string());

        let schedule: Schedule = config.try_into().unwrap();

        assert_eq!(schedule.expression(), "0 9 * * *");
        assert_eq!(schedule.timezone(), "UTC");
    }

    #[test]
    fn test_schedule_config_detailed_to_schedule() {
        let config = ScheduleConfig::Detailed {
            cron: "0 9 * * *".to_string(),
            timezone: Some("America/New_York".to_string()),
        };

        let schedule: Schedule = config.try_into().unwrap();

        assert_eq!(schedule.expression(), "0 9 * * *");
        assert_eq!(schedule.timezone(), "America/New_York");
    }

    #[test]
    fn test_schedule_config_detailed_without_timezone_to_schedule() {
        let config = ScheduleConfig::Detailed {
            cron: "@hourly".to_string(),
            timezone: None,
        };

        let schedule: Schedule = config.try_into().unwrap();

        assert_eq!(schedule.expression(), "@hourly");
        assert_eq!(schedule.timezone(), "UTC");
    }

    #[test]
    fn test_schedule_config_invalid_cron_returns_error() {
        let config = ScheduleConfig::Simple("invalid cron".to_string());

        let result: Result<Schedule, ConfigError> = config.try_into();

        assert!(result.is_err());
        assert!(matches!(result, Err(ConfigError::InvalidConfig(_))));
    }

    #[test]
    fn test_schedule_config_invalid_timezone_returns_error() {
        let config = ScheduleConfig::Detailed {
            cron: "0 * * * *".to_string(),
            timezone: Some("Invalid/Timezone".to_string()),
        };

        let result: Result<Schedule, ConfigError> = config.try_into();

        assert!(result.is_err());
        assert!(matches!(result, Err(ConfigError::InvalidConfig(_))));
    }

    #[test]
    fn test_task_condition_config_all_success_to_task_condition() {
        let config = TaskConditionConfig::AllSuccess;

        let condition: TaskCondition = config.into();

        assert_eq!(condition, TaskCondition::AllSuccess);
    }

    #[test]
    fn test_task_condition_config_on_failure_to_task_condition() {
        let config = TaskConditionConfig::OnFailure;

        let condition: TaskCondition = config.into();

        assert_eq!(condition, TaskCondition::OnFailure);
    }

    #[test]
    fn test_task_condition_config_all_done_to_task_condition() {
        let config = TaskConditionConfig::AllDone;

        let condition: TaskCondition = config.into();

        assert_eq!(condition, TaskCondition::AllDone);
    }

    #[test]
    fn test_job_dependency_condition_config_last_success_to_dependency_condition() {
        let config = JobDependencyConditionConfig::LastSuccess;

        let condition: DependencyCondition = config.try_into().unwrap();

        assert_eq!(condition, DependencyCondition::LastSuccess);
    }

    #[test]
    fn test_job_dependency_condition_config_last_complete_to_dependency_condition() {
        let config = JobDependencyConditionConfig::LastComplete;

        let condition: DependencyCondition = config.try_into().unwrap();

        assert_eq!(condition, DependencyCondition::LastComplete);
    }

    #[test]
    fn test_job_dependency_condition_config_within_window_to_dependency_condition() {
        let config = JobDependencyConditionConfig::WithinWindow { seconds: 3600 };

        let condition: DependencyCondition = config.try_into().unwrap();

        assert_eq!(
            condition,
            DependencyCondition::WithinWindow(Duration::from_secs(3600))
        );
    }

    #[test]
    fn test_conversion_roundtrip_retry_config() {
        // Create a RetryConfig, convert to RetryPolicy, verify fields match
        let config = RetryConfig {
            max_attempts: 3,
            delay_secs: 45,
            condition: RetryConditionConfig::TransientOnly,
        };

        let policy: RetryPolicy = config.clone().try_into().unwrap();

        assert_eq!(policy.max_attempts, config.max_attempts);
        assert_eq!(policy.delay, Duration::from_secs(config.delay_secs));
        assert_eq!(policy.retry_on, RetryCondition::TransientOnly);
    }

    #[test]
    fn test_conversion_schedule_with_interval() {
        let config = ScheduleConfig::Simple("@every 5m".to_string());

        let schedule: Schedule = config.try_into().unwrap();

        assert_eq!(schedule.expression(), "@every 5m");
    }
}
