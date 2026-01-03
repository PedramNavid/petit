//! YAML configuration parsing.
//!
//! Parses job definitions and global configuration from YAML files.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
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

        // Check that task dependencies reference valid tasks, no self-dependencies, and no duplicates
        for task in &config.tasks {
            let mut seen_deps = HashSet::new();
            for dep in &task.depends_on {
                // Check self-dependency
                if dep == &task.id {
                    return Err(ConfigError::InvalidConfig(format!(
                        "task '{}' cannot depend on itself",
                        task.id
                    )));
                }
                // Check dependency exists
                if !task_ids.contains(dep.as_str()) {
                    return Err(ConfigError::InvalidConfig(format!(
                        "task '{}' depends on unknown task '{}'",
                        task.id, dep
                    )));
                }
                // Check for duplicate dependencies
                if !seen_deps.insert(dep) {
                    return Err(ConfigError::InvalidConfig(format!(
                        "task '{}' has duplicate dependency '{}'",
                        task.id, dep
                    )));
                }
            }
        }

        // Check for cycles using topological sort (Kahn's algorithm)
        Self::validate_no_cycles(config)?;

        Ok(())
    }

    /// Validate that there are no cycles in the task dependency graph.
    fn validate_no_cycles(config: &JobConfig) -> Result<(), ConfigError> {
        // Build dependency graph
        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        let mut reverse_deps: HashMap<&str, Vec<&str>> = HashMap::new();

        // Initialize in-degrees for all tasks
        for task in &config.tasks {
            in_degree.insert(&task.id, task.depends_on.len());
            reverse_deps.insert(&task.id, Vec::new());
        }

        // Build reverse dependency map
        for task in &config.tasks {
            for dep in &task.depends_on {
                reverse_deps.entry(dep.as_str()).or_default().push(&task.id);
            }
        }

        // Kahn's algorithm: start with nodes that have no dependencies
        let mut queue: VecDeque<&str> = in_degree
            .iter()
            .filter(|(_, degree)| **degree == 0)
            .map(|(id, _)| *id)
            .collect();

        let mut visited_count = 0;

        while let Some(id) = queue.pop_front() {
            visited_count += 1;

            // Reduce in-degree for all downstream tasks
            if let Some(downstream) = reverse_deps.get(id) {
                for next in downstream {
                    if let Some(degree) = in_degree.get_mut(next) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push_back(next);
                        }
                    }
                }
            }
        }

        // If we didn't visit all nodes, there's a cycle
        if visited_count != config.tasks.len() {
            // Find tasks that are part of the cycle (those with non-zero in-degree)
            let cycle_tasks: Vec<&str> = in_degree
                .iter()
                .filter(|(_, degree)| **degree > 0)
                .map(|(id, _)| *id)
                .collect();

            return Err(ConfigError::InvalidConfig(format!(
                "dependency cycle detected involving tasks: {}",
                cycle_tasks.join(", ")
            )));
        }

        Ok(())
    }

    /// Convert retry config to Duration.
    pub fn retry_delay(config: &RetryConfig) -> Duration {
        Duration::from_secs(config.delay_secs)
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

    #[test]
    fn test_validation_error_self_dependency() {
        let yaml = r#"
id: self_dep_job
name: Self Dependency Job
tasks:
  - id: task1
    type: command
    command: echo
    depends_on: [task1]
"#;
        let result = YamlLoader::parse_job_config(yaml);
        assert!(matches!(result, Err(ConfigError::InvalidConfig(_))));
        if let Err(ConfigError::InvalidConfig(msg)) = result {
            assert!(msg.contains("cannot depend on itself"));
            assert!(msg.contains("task1"));
        }
    }

    #[test]
    fn test_validation_error_duplicate_dependency() {
        let yaml = r#"
id: dup_dep_job
name: Duplicate Dependency Job
tasks:
  - id: task1
    type: command
    command: echo
  - id: task2
    type: command
    command: echo
    depends_on: [task1, task1]
"#;
        let result = YamlLoader::parse_job_config(yaml);
        assert!(matches!(result, Err(ConfigError::InvalidConfig(_))));
        if let Err(ConfigError::InvalidConfig(msg)) = result {
            assert!(msg.contains("duplicate dependency"));
            assert!(msg.contains("task2"));
        }
    }

    #[test]
    fn test_validation_error_simple_cycle() {
        let yaml = r#"
id: cycle_job
name: Cycle Job
tasks:
  - id: task1
    type: command
    command: echo
    depends_on: [task2]
  - id: task2
    type: command
    command: echo
    depends_on: [task1]
"#;
        let result = YamlLoader::parse_job_config(yaml);
        assert!(matches!(result, Err(ConfigError::InvalidConfig(_))));
        if let Err(ConfigError::InvalidConfig(msg)) = result {
            assert!(msg.contains("cycle detected"));
        }
    }

    #[test]
    fn test_validation_error_complex_cycle() {
        let yaml = r#"
id: complex_cycle_job
name: Complex Cycle Job
tasks:
  - id: task1
    type: command
    command: echo
  - id: task2
    type: command
    command: echo
    depends_on: [task1]
  - id: task3
    type: command
    command: echo
    depends_on: [task2]
  - id: task4
    type: command
    command: echo
    depends_on: [task3, task1]
  - id: task5
    type: command
    command: echo
    depends_on: [task4]
  - id: task6
    type: command
    command: echo
    depends_on: [task5, task2]
"#;
        // This should pass - it's a valid DAG
        let result = YamlLoader::parse_job_config(yaml);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validation_error_three_way_cycle() {
        let yaml = r#"
id: three_way_cycle_job
name: Three Way Cycle Job
tasks:
  - id: task1
    type: command
    command: echo
    depends_on: [task3]
  - id: task2
    type: command
    command: echo
    depends_on: [task1]
  - id: task3
    type: command
    command: echo
    depends_on: [task2]
"#;
        let result = YamlLoader::parse_job_config(yaml);
        assert!(matches!(result, Err(ConfigError::InvalidConfig(_))));
        if let Err(ConfigError::InvalidConfig(msg)) = result {
            assert!(msg.contains("cycle detected"));
            // All three tasks should be mentioned as part of the cycle
            assert!(msg.contains("task1") || msg.contains("task2") || msg.contains("task3"));
        }
    }

    #[test]
    fn test_validation_valid_complex_dag() {
        let yaml = r#"
id: complex_dag_job
name: Complex DAG Job
tasks:
  - id: extract1
    type: command
    command: ./extract1.sh
  - id: extract2
    type: command
    command: ./extract2.sh
  - id: transform1
    type: command
    command: ./transform1.sh
    depends_on: [extract1]
  - id: transform2
    type: command
    command: ./transform2.sh
    depends_on: [extract2]
  - id: merge
    type: command
    command: ./merge.sh
    depends_on: [transform1, transform2]
  - id: load
    type: command
    command: ./load.sh
    depends_on: [merge]
"#;
        let result = YamlLoader::parse_job_config(yaml);
        assert!(result.is_ok());
    }
}
