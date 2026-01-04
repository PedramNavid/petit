//! Job builder from YAML configuration.
//!
//! This module converts JobConfig into runnable Job instances with DAGs.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::core::dag::{DagBuilder, TaskCondition};
use crate::core::environment::Environment;
use crate::core::job::Job;
use crate::core::retry::{RetryCondition, RetryPolicy};
use crate::core::schedule::Schedule;
use crate::execution::CommandTask;

use super::yaml::{
    ConfigError, JobConfig, RetryConditionConfig, TaskConditionConfig, TaskTypeConfig, YamlLoader,
};

/// Builder for creating Jobs from YAML configuration.
///
/// Converts parsed YAML job configurations into runnable [`Job`] instances
/// with fully constructed DAGs, task executors, and scheduling information.
pub struct JobConfigBuilder;

impl JobConfigBuilder {
    /// Build a Job from a JobConfig.
    ///
    /// This method converts a parsed YAML configuration into a runnable Job by:
    /// - Building a DAG from task definitions and dependencies
    /// - Creating task executors (command, python, etc.)
    /// - Applying retry policies, timeouts, and conditions
    /// - Setting up schedules and concurrency limits
    ///
    /// # Examples
    ///
    /// ```
    /// use petit::config::{YamlLoader, JobConfigBuilder};
    ///
    /// let yaml = r#"
    /// id: example
    /// name: Example Job
    /// tasks:
    ///   - id: task1
    ///     type: command
    ///     command: echo
    ///     args: [hello]
    /// "#;
    ///
    /// let config = YamlLoader::parse_job_config(yaml)?;
    /// let job = JobConfigBuilder::build(config)?;
    ///
    /// assert_eq!(job.id().as_str(), "example");
    /// assert_eq!(job.dag().len(), 1);
    /// # Ok::<(), petit::config::ConfigError>(())
    /// ```
    pub fn build(config: JobConfig) -> Result<Job, ConfigError> {
        // Build the DAG from task configs
        let mut dag_builder = DagBuilder::new(&config.id, &config.name);

        for task_config in &config.tasks {
            let task = Self::build_task(&config.name, task_config)?;
            let condition = task_config
                .condition
                .as_ref()
                .map(Self::convert_condition)
                .unwrap_or(TaskCondition::AllSuccess);

            let deps: Vec<&str> = task_config.depends_on.iter().map(|s| s.as_str()).collect();
            dag_builder = dag_builder.add_task_with_deps_and_condition(task, &deps, condition);
        }

        let dag = dag_builder
            .build()
            .map_err(|e| ConfigError::InvalidConfig(format!("Job '{}': {}", config.name, e)))?;

        // Create the job
        let mut job = Job::new(config.id.clone(), &config.name, dag);

        // Add schedule if present
        if let Some(schedule_config) = &config.schedule {
            let cron_expr = schedule_config.cron();
            let tz = schedule_config.timezone().unwrap_or("UTC");
            let schedule = Schedule::with_timezone(cron_expr, tz).map_err(|e| {
                ConfigError::InvalidConfig(format!(
                    "Job '{}': invalid schedule: {} {}",
                    config.name, cron_expr, e
                ))
            })?;
            job = job.with_schedule(schedule);
        }

        // Add job-level config
        let mut job_config_map = std::collections::HashMap::new();
        for (key, value) in &config.config {
            // Convert serde_yaml::Value to serde_json::Value
            let json_value = serde_json::to_value(value).map_err(|e| {
                ConfigError::InvalidConfig(format!(
                    "Job '{}': failed to convert config key '{}': {}",
                    config.name, key, e
                ))
            })?;
            job_config_map.insert(key.clone(), json_value);
        }
        if !job_config_map.is_empty() {
            job = job.with_config(job_config_map);
        }

        // Set max concurrency
        if let Some(max) = config.max_concurrency {
            job = job.with_max_concurrency(max);
        }

        // Set enabled state
        job = job.with_enabled(config.enabled);

        Ok(job)
    }

    /// Build a Task from TaskConfig.
    fn build_task(
        job_name: &str,
        config: &super::yaml::TaskConfig,
    ) -> Result<Arc<dyn crate::core::task::Task>, ConfigError> {
        match &config.task_type {
            TaskTypeConfig::Command {
                command,
                args,
                working_dir,
                timeout_secs,
            } => {
                let mut builder = CommandTask::builder(command).name(&config.id);

                for arg in args {
                    builder = builder.arg(arg);
                }

                // Add environment variables
                let mut env = Environment::new();
                for (key, value) in &config.environment {
                    env = env.with_var(key, value);
                }
                builder = builder.environment(env);

                // Set working directory
                if let Some(dir) = working_dir {
                    builder = builder.working_dir(dir);
                }

                // Set timeout
                if let Some(secs) = timeout_secs {
                    builder = builder.timeout(Duration::from_secs(*secs));
                }

                // Set retry policy
                if let Some(retry_config) = &config.retry {
                    let policy = Self::build_retry_policy(retry_config);
                    builder = builder.retry_policy(policy);
                }

                Ok(Arc::new(builder.build()))
            }
            TaskTypeConfig::Python { script, inline } => {
                // For Python tasks, we run python with the script
                let mut builder = if *inline {
                    CommandTask::builder("python")
                        .name(&config.id)
                        .arg("-c")
                        .arg(script)
                } else {
                    CommandTask::builder("python").name(&config.id).arg(script)
                };

                // Add environment variables
                let mut env = Environment::new();
                for (key, value) in &config.environment {
                    env = env.with_var(key, value);
                }
                builder = builder.environment(env);

                // Set retry policy
                if let Some(retry_config) = &config.retry {
                    let policy = Self::build_retry_policy(retry_config);
                    builder = builder.retry_policy(policy);
                }

                Ok(Arc::new(builder.build()))
            }
            TaskTypeConfig::Custom { handler, .. } => Err(ConfigError::InvalidConfig(format!(
                "Job '{}': Task '{}': custom task type '{}' is not supported yet",
                job_name, config.id, handler
            ))),
        }
    }

    /// Build a RetryPolicy from RetryConfig.
    fn build_retry_policy(config: &super::yaml::RetryConfig) -> RetryPolicy {
        let delay = Duration::from_secs(config.delay_secs);
        let condition = match config.condition {
            RetryConditionConfig::Always => RetryCondition::Always,
            RetryConditionConfig::TransientOnly => RetryCondition::TransientOnly,
            RetryConditionConfig::Never => RetryCondition::Never,
        };

        RetryPolicy::fixed(config.max_attempts, delay).with_condition(condition)
    }

    /// Convert TaskConditionConfig to TaskCondition.
    fn convert_condition(config: &TaskConditionConfig) -> TaskCondition {
        match config {
            TaskConditionConfig::AllSuccess => TaskCondition::AllSuccess,
            TaskConditionConfig::OnFailure => TaskCondition::OnFailure,
            TaskConditionConfig::AllDone => TaskCondition::AllDone,
        }
    }
}

/// Load all job configurations from a directory.
///
/// Reads all `.yaml` and `.yml` files in the directory, parses them as job configurations,
/// and builds runnable Job instances. Files that are not valid YAML or do not conform
/// to the job configuration schema will cause an error.
///
/// # Examples
///
/// ```no_run
/// use petit::config::load_jobs_from_directory;
///
/// // Load all jobs from the jobs directory
/// let jobs = load_jobs_from_directory("./jobs")?;
///
/// println!("Loaded {} jobs:", jobs.len());
/// for job in &jobs {
///     println!("  - {} ({})", job.name(), job.id());
///     if job.is_scheduled() {
///         println!("    Scheduled job");
///     }
/// }
/// # Ok::<(), petit::config::ConfigError>(())
/// ```
pub fn load_jobs_from_directory(dir: impl AsRef<Path>) -> Result<Vec<Job>, ConfigError> {
    let dir = dir.as_ref();
    let mut jobs = Vec::new();

    let entries = std::fs::read_dir(dir).map_err(|source| ConfigError::DirReadError {
        path: dir.to_path_buf(),
        source,
    })?;

    for entry in entries {
        let entry = entry.map_err(|source| ConfigError::DirReadError {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();

        // Only process .yaml and .yml files
        if let Some(ext) = path.extension()
            && (ext == "yaml" || ext == "yml")
        {
            let config = YamlLoader::load_job_config(&path)?;
            let job = JobConfigBuilder::build(config)?;
            jobs.push(job);
        }
    }

    Ok(jobs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_simple_job() {
        let yaml = r#"
id: simple_job
name: Simple Job
tasks:
  - id: hello
    type: command
    command: echo
    args: ["Hello, World!"]
"#;

        let config = YamlLoader::parse_job_config(yaml).unwrap();
        let job = JobConfigBuilder::build(config).unwrap();

        assert_eq!(job.id().as_str(), "simple_job");
        assert_eq!(job.name(), "Simple Job");
        assert_eq!(job.dag().len(), 1);
    }

    #[test]
    fn test_build_job_with_schedule() {
        let yaml = r#"
id: scheduled_job
name: Scheduled Job
schedule: "0 0 * * * *"
tasks:
  - id: task1
    type: command
    command: echo
    args: ["scheduled"]
"#;

        let config = YamlLoader::parse_job_config(yaml).unwrap();
        let job = JobConfigBuilder::build(config).unwrap();

        assert!(job.is_scheduled());
    }

    #[test]
    fn test_build_job_with_dependencies() {
        let yaml = r#"
id: pipeline
name: Pipeline
tasks:
  - id: extract
    type: command
    command: echo
    args: ["extract"]
  - id: transform
    type: command
    command: echo
    args: ["transform"]
    depends_on: [extract]
  - id: load
    type: command
    command: echo
    args: ["load"]
    depends_on: [transform]
"#;

        let config = YamlLoader::parse_job_config(yaml).unwrap();
        let job = JobConfigBuilder::build(config).unwrap();

        assert_eq!(job.dag().len(), 3);

        let order = job.dag().topological_sort().unwrap();
        let names: Vec<&str> = order.iter().map(|id| id.as_str()).collect();
        assert_eq!(names, vec!["extract", "transform", "load"]);
    }

    #[test]
    fn test_build_job_with_conditions() {
        let yaml = r#"
id: conditional_job
name: Conditional Job
tasks:
  - id: main
    type: command
    command: ./run.sh
  - id: cleanup
    type: command
    command: ./cleanup.sh
    depends_on: [main]
    condition: all_done
  - id: notify_error
    type: command
    command: ./notify.sh
    depends_on: [main]
    condition: on_failure
"#;

        let config = YamlLoader::parse_job_config(yaml).unwrap();
        let job = JobConfigBuilder::build(config).unwrap();

        assert_eq!(job.dag().len(), 3);
    }

    #[test]
    fn test_build_job_with_retry() {
        let yaml = r#"
id: retry_job
name: Retry Job
tasks:
  - id: flaky
    type: command
    command: ./flaky.sh
    retry:
      max_attempts: 5
      delay_secs: 10
      condition: always
"#;

        let config = YamlLoader::parse_job_config(yaml).unwrap();
        let job = JobConfigBuilder::build(config).unwrap();

        assert_eq!(job.dag().len(), 1);
    }

    #[test]
    fn test_build_python_task() {
        let yaml = r#"
id: python_job
name: Python Job
tasks:
  - id: script
    type: python
    script: print("hello")
    inline: true
"#;

        let config = YamlLoader::parse_job_config(yaml).unwrap();
        let job = JobConfigBuilder::build(config).unwrap();

        assert_eq!(job.dag().len(), 1);
    }

    #[test]
    fn test_build_job_with_config() {
        let yaml = r#"
id: config_job
name: Config Job
config:
  batch_size: 1000
  debug: true
tasks:
  - id: task1
    type: command
    command: echo
    args: ["test"]
"#;

        let config = YamlLoader::parse_job_config(yaml).unwrap();
        let job = JobConfigBuilder::build(config).unwrap();

        assert_eq!(job.get_config::<i32>("batch_size"), Some(1000));
        assert_eq!(job.get_config::<bool>("debug"), Some(true));
    }

    #[test]
    fn test_build_disabled_job() {
        let yaml = r#"
id: disabled_job
name: Disabled Job
enabled: false
tasks:
  - id: task1
    type: command
    command: echo
    args: ["test"]
"#;

        let config = YamlLoader::parse_job_config(yaml).unwrap();
        let job = JobConfigBuilder::build(config).unwrap();

        assert!(!job.is_enabled());
    }

    #[test]
    fn test_dir_read_error_includes_path() {
        let nonexistent_dir = "/nonexistent/jobs/directory";
        let result = super::load_jobs_from_directory(nonexistent_dir);

        match result {
            Err(ConfigError::DirReadError { path, source }) => {
                assert_eq!(path.to_str(), Some(nonexistent_dir));
                assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
            }
            _ => panic!("Expected DirReadError with path context"),
        }
    }
}
