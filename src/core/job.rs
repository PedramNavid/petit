//! Job definition with schedule and dependencies.
//!
//! A Job combines a DAG with scheduling and cross-job dependencies.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

use super::dag::Dag;
use super::schedule::Schedule;
use super::types::JobId;

/// Errors that can occur when working with jobs.
#[derive(Debug, Error)]
pub enum JobError {
    /// Invalid DAG configuration.
    #[error("invalid DAG: {0}")]
    InvalidDag(String),

    /// Invalid dependency.
    #[error("invalid dependency: {0}")]
    InvalidDependency(String),

    /// Missing dependency.
    #[error("missing job dependency: {0}")]
    MissingDependency(String),

    /// Invalid schedule.
    #[error("invalid schedule: {0}")]
    InvalidSchedule(String),

    /// Invalid configuration.
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
}

/// Condition for cross-job dependencies.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DependencyCondition {
    /// Dependent job's last run must have succeeded.
    #[default]
    LastSuccess,
    /// Dependent job must have succeeded within the given duration.
    WithinWindow(Duration),
    /// Dependent job must have completed (success or failure).
    LastComplete,
}

/// A cross-job dependency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobDependency {
    /// The job ID this depends on.
    job_id: JobId,
    /// The condition that must be met.
    condition: DependencyCondition,
}

impl JobDependency {
    /// Create a new job dependency with LastSuccess condition.
    pub fn new(job_id: JobId) -> Self {
        Self {
            job_id,
            condition: DependencyCondition::LastSuccess,
        }
    }

    /// Create a dependency with a specific condition.
    pub fn with_condition(job_id: JobId, condition: DependencyCondition) -> Self {
        Self { job_id, condition }
    }

    /// Get the dependent job ID.
    pub fn job_id(&self) -> &JobId {
        &self.job_id
    }

    /// Get the dependency condition.
    pub fn condition(&self) -> &DependencyCondition {
        &self.condition
    }
}

/// A job definition combining DAG, schedule, and configuration.
#[derive(Clone)]
pub struct Job {
    /// Unique job identifier.
    id: JobId,
    /// Human-readable name.
    name: String,
    /// The DAG defining task execution order.
    dag: Arc<Dag>,
    /// Optional schedule for automatic execution.
    schedule: Option<Schedule>,
    /// Cross-job dependencies.
    dependencies: Vec<JobDependency>,
    /// Job-level configuration passed to all tasks.
    config: HashMap<String, Value>,
    /// Maximum concurrent runs of this job (None = unlimited).
    max_concurrency: Option<usize>,
    /// Whether the job is enabled.
    enabled: bool,
}

impl std::fmt::Debug for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Job")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("enabled", &self.enabled)
            .field("schedule", &self.schedule)
            .field("dag_id", self.dag.id())
            .field("dependencies", &self.dependencies)
            .field("max_concurrency", &self.max_concurrency)
            .finish()
    }
}

impl Job {
    /// Create a new job with the given ID, name, and DAG.
    pub fn new(id: impl Into<JobId>, name: impl Into<String>, dag: Dag) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            dag: Arc::new(dag),
            schedule: None,
            dependencies: Vec::new(),
            config: HashMap::new(),
            max_concurrency: None,
            enabled: true,
        }
    }

    /// Set the schedule for this job.
    pub fn with_schedule(mut self, schedule: Schedule) -> Self {
        self.schedule = Some(schedule);
        self
    }

    /// Add a cross-job dependency.
    pub fn with_dependency(mut self, dependency: JobDependency) -> Self {
        self.dependencies.push(dependency);
        self
    }

    /// Add multiple cross-job dependencies.
    pub fn with_dependencies(mut self, dependencies: Vec<JobDependency>) -> Self {
        self.dependencies.extend(dependencies);
        self
    }

    /// Set job-level configuration.
    pub fn with_config(mut self, config: HashMap<String, Value>) -> Self {
        self.config = config;
        self
    }

    /// Add a single configuration value.
    pub fn with_config_value(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.config.insert(key.into(), value.into());
        self
    }

    /// Set the maximum concurrent runs.
    ///
    /// # Panics
    ///
    /// Panics if `max` is zero. Use `JobBuilder` if you need error handling.
    pub fn with_max_concurrency(mut self, max: usize) -> Self {
        assert!(max > 0, "max_concurrency cannot be zero");
        self.max_concurrency = Some(max);
        self
    }

    /// Set whether the job is enabled.
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Get the job ID.
    pub fn id(&self) -> &JobId {
        &self.id
    }

    /// Get the job name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the DAG.
    pub fn dag(&self) -> &Dag {
        &self.dag
    }

    /// Get the schedule, if any.
    pub fn schedule(&self) -> Option<&Schedule> {
        self.schedule.as_ref()
    }

    /// Get the cross-job dependencies.
    pub fn dependencies(&self) -> &[JobDependency] {
        &self.dependencies
    }

    /// Get the job-level configuration.
    pub fn config(&self) -> &HashMap<String, Value> {
        &self.config
    }

    /// Get a specific configuration value.
    pub fn get_config<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.config
            .get(key)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// Get the maximum concurrent runs.
    pub fn max_concurrency(&self) -> Option<usize> {
        self.max_concurrency
    }

    /// Check if the job is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Check if this job has a schedule (vs manual trigger only).
    pub fn is_scheduled(&self) -> bool {
        self.schedule.is_some()
    }

    /// Validate the job configuration.
    ///
    /// Checks that:
    /// - The DAG is valid
    /// - No self-dependencies exist
    /// - All dependency job IDs are provided in known_jobs
    pub fn validate(&self, known_jobs: &HashSet<JobId>) -> Result<(), JobError> {
        // Validate DAG
        self.dag
            .validate()
            .map_err(|e| JobError::InvalidDag(e.to_string()))?;

        // Validate dependencies
        for dep in &self.dependencies {
            // Check for self-dependency
            if dep.job_id == self.id {
                return Err(JobError::InvalidDependency(
                    "job cannot depend on itself".to_string(),
                ));
            }

            // Check dependency exists
            if !known_jobs.contains(&dep.job_id) {
                return Err(JobError::MissingDependency(dep.job_id.to_string()));
            }
        }

        Ok(())
    }
}

/// Builder for creating jobs with a fluent API.
pub struct JobBuilder {
    id: JobId,
    name: String,
    dag: Option<Dag>,
    schedule: Option<Schedule>,
    dependencies: Vec<JobDependency>,
    config: HashMap<String, Value>,
    max_concurrency: Option<usize>,
    enabled: bool,
}

impl JobBuilder {
    /// Create a new job builder.
    pub fn new(id: impl Into<JobId>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            dag: None,
            schedule: None,
            dependencies: Vec::new(),
            config: HashMap::new(),
            max_concurrency: None,
            enabled: true,
        }
    }

    /// Set the DAG for this job.
    pub fn dag(mut self, dag: Dag) -> Self {
        self.dag = Some(dag);
        self
    }

    /// Set the schedule for this job.
    pub fn schedule(mut self, schedule: Schedule) -> Self {
        self.schedule = Some(schedule);
        self
    }

    /// Add a dependency on another job.
    pub fn depends_on(mut self, job_id: impl Into<JobId>) -> Self {
        self.dependencies.push(JobDependency::new(job_id.into()));
        self
    }

    /// Add a dependency with a specific condition.
    pub fn depends_on_with_condition(
        mut self,
        job_id: impl Into<JobId>,
        condition: DependencyCondition,
    ) -> Self {
        self.dependencies
            .push(JobDependency::with_condition(job_id.into(), condition));
        self
    }

    /// Add a configuration value.
    pub fn config(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.config.insert(key.into(), value.into());
        self
    }

    /// Set maximum concurrent runs.
    ///
    /// # Panics
    ///
    /// Panics if `max` is zero.
    pub fn max_concurrency(mut self, max: usize) -> Self {
        assert!(max > 0, "max_concurrency cannot be zero");
        self.max_concurrency = Some(max);
        self
    }

    /// Set whether the job is enabled.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Build the job.
    pub fn build(self) -> Result<Job, JobError> {
        let dag = self
            .dag
            .ok_or(JobError::InvalidDag("DAG is required".into()))?;

        // Validate max_concurrency is not zero
        if self.max_concurrency == Some(0) {
            return Err(JobError::InvalidConfig(
                "max_concurrency cannot be zero".into(),
            ));
        }

        Ok(Job {
            id: self.id,
            name: self.name,
            dag: Arc::new(dag),
            schedule: self.schedule,
            dependencies: self.dependencies,
            config: self.config,
            max_concurrency: self.max_concurrency,
            enabled: self.enabled,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TaskContext;
    use crate::core::dag::DagBuilder;
    use crate::core::task::{Task, TaskError};
    use async_trait::async_trait;

    struct DummyTask {
        name: String,
    }

    impl DummyTask {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
            }
        }
    }

    #[async_trait]
    impl Task for DummyTask {
        fn name(&self) -> &str {
            &self.name
        }

        async fn execute(&self, _ctx: &mut TaskContext) -> Result<(), TaskError> {
            Ok(())
        }
    }

    fn create_simple_dag() -> Dag {
        DagBuilder::new("test_dag", "Test DAG")
            .add_task(Arc::new(DummyTask::new("task1")))
            .build()
            .unwrap()
    }

    fn create_etl_dag() -> Dag {
        DagBuilder::new("etl", "ETL Pipeline")
            .add_task(Arc::new(DummyTask::new("extract")))
            .add_task_with_deps(Arc::new(DummyTask::new("transform")), &["extract"])
            .add_task_with_deps(Arc::new(DummyTask::new("load")), &["transform"])
            .build()
            .unwrap()
    }

    #[test]
    fn test_create_job_with_dag_and_schedule() {
        let dag = create_simple_dag();
        let schedule = Schedule::new("0 * * * *").unwrap();

        let job = Job::new("daily_job", "Daily Job", dag).with_schedule(schedule);

        assert_eq!(job.id().as_str(), "daily_job");
        assert_eq!(job.name(), "Daily Job");
        assert!(job.is_scheduled());
        assert!(job.schedule().is_some());
    }

    #[test]
    fn test_job_without_schedule_manual_trigger_only() {
        let dag = create_simple_dag();
        let job = Job::new("manual_job", "Manual Job", dag);

        assert!(!job.is_scheduled());
        assert!(job.schedule().is_none());
    }

    #[test]
    fn test_job_with_cross_job_dependency() {
        let dag = create_simple_dag();
        let job = Job::new("dependent_job", "Dependent Job", dag)
            .with_dependency(JobDependency::new(JobId::new("upstream_job")));

        assert_eq!(job.dependencies().len(), 1);
        assert_eq!(job.dependencies()[0].job_id().as_str(), "upstream_job");
        assert_eq!(
            job.dependencies()[0].condition(),
            &DependencyCondition::LastSuccess
        );
    }

    #[test]
    fn test_job_dependency_conditions() {
        let dag = create_simple_dag();

        // LastSuccess (default)
        let dep1 = JobDependency::new(JobId::new("job1"));
        assert_eq!(dep1.condition(), &DependencyCondition::LastSuccess);

        // WithinWindow
        let dep2 = JobDependency::with_condition(
            JobId::new("job2"),
            DependencyCondition::WithinWindow(Duration::from_secs(3600)),
        );
        assert!(matches!(
            dep2.condition(),
            DependencyCondition::WithinWindow(_)
        ));

        // LastComplete
        let dep3 =
            JobDependency::with_condition(JobId::new("job3"), DependencyCondition::LastComplete);
        assert_eq!(dep3.condition(), &DependencyCondition::LastComplete);

        let job = Job::new("test", "Test", dag)
            .with_dependency(dep1)
            .with_dependency(dep2)
            .with_dependency(dep3);

        assert_eq!(job.dependencies().len(), 3);
    }

    #[test]
    fn test_job_level_config_passed_to_tasks() {
        let dag = create_etl_dag();

        let mut config = HashMap::new();
        config.insert("batch_size".to_string(), serde_json::json!(1000));
        config.insert("db_host".to_string(), serde_json::json!("localhost"));

        let job = Job::new("etl_job", "ETL Job", dag).with_config(config);

        assert_eq!(job.get_config::<i32>("batch_size"), Some(1000));
        assert_eq!(
            job.get_config::<String>("db_host"),
            Some("localhost".to_string())
        );
        assert_eq!(job.get_config::<String>("missing"), None);
    }

    #[test]
    fn test_job_concurrency_limit() {
        let dag = create_simple_dag();

        // No limit by default
        let job1 = Job::new("job1", "Job 1", dag.clone());
        assert_eq!(job1.max_concurrency(), None);

        // With limit
        let job2 = Job::new("job2", "Job 2", dag).with_max_concurrency(1);
        assert_eq!(job2.max_concurrency(), Some(1));
    }

    #[test]
    fn test_validate_job_with_valid_dag() {
        let dag = create_etl_dag();
        let job = Job::new("valid_job", "Valid Job", dag);

        let known_jobs = HashSet::new();
        let result = job.validate(&known_jobs);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_job_missing_dependency() {
        let dag = create_simple_dag();
        let job = Job::new("dep_job", "Dependent Job", dag)
            .with_dependency(JobDependency::new(JobId::new("nonexistent")));

        let mut known_jobs = HashSet::new();
        known_jobs.insert(JobId::new("other_job"));
        let result = job.validate(&known_jobs);
        assert!(matches!(result, Err(JobError::MissingDependency(_))));
    }

    #[test]
    fn test_validate_job_with_existing_dependency() {
        let dag = create_simple_dag();
        let job = Job::new("dep_job", "Dependent Job", dag)
            .with_dependency(JobDependency::new(JobId::new("upstream")));

        let mut known_jobs = HashSet::new();
        known_jobs.insert(JobId::new("upstream"));
        known_jobs.insert(JobId::new("other"));
        let result = job.validate(&known_jobs);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_job_rejects_self_dependency() {
        let dag = create_simple_dag();
        let job_id = JobId::new("self_dep_job");
        let job = Job::new(job_id.clone(), "Self Dependent Job", dag)
            .with_dependency(JobDependency::new(job_id.clone()));

        // Even if the job is in known_jobs, self-dependency should be rejected
        let known_jobs: HashSet<JobId> = [job_id].into_iter().collect();
        let result = job.validate(&known_jobs);
        assert!(matches!(result, Err(JobError::InvalidDependency(_))));

        if let Err(JobError::InvalidDependency(msg)) = result {
            assert!(msg.contains("cannot depend on itself"));
        }
    }

    #[test]
    fn test_job_builder() {
        let dag = create_etl_dag();
        let schedule = Schedule::new("@daily").unwrap();

        let job = JobBuilder::new("built_job", "Built Job")
            .dag(dag)
            .schedule(schedule)
            .depends_on("upstream_job")
            .config("key", "value")
            .max_concurrency(2)
            .enabled(true)
            .build()
            .unwrap();

        assert_eq!(job.id().as_str(), "built_job");
        assert!(job.is_scheduled());
        assert_eq!(job.dependencies().len(), 1);
        assert_eq!(job.max_concurrency(), Some(2));
        assert!(job.is_enabled());
    }

    #[test]
    fn test_job_builder_without_dag_fails() {
        let result = JobBuilder::new("no_dag", "No DAG").build();
        assert!(matches!(result, Err(JobError::InvalidDag(_))));
    }

    #[test]
    fn test_job_builder_with_condition() {
        let dag = create_simple_dag();

        let job = JobBuilder::new("job", "Job")
            .dag(dag)
            .depends_on_with_condition(
                "upstream",
                DependencyCondition::WithinWindow(Duration::from_secs(7200)),
            )
            .build()
            .unwrap();

        assert!(matches!(
            job.dependencies()[0].condition(),
            DependencyCondition::WithinWindow(_)
        ));
    }

    #[test]
    fn test_job_enabled_disabled() {
        let dag = create_simple_dag();

        let enabled_job = Job::new("enabled", "Enabled", dag.clone());
        assert!(enabled_job.is_enabled());

        let disabled_job = Job::new("disabled", "Disabled", dag).with_enabled(false);
        assert!(!disabled_job.is_enabled());
    }

    #[test]
    fn test_job_with_config_value() {
        let dag = create_simple_dag();

        let job = Job::new("config_job", "Config Job", dag)
            .with_config_value("timeout", 30)
            .with_config_value("retries", 3);

        assert_eq!(job.get_config::<i32>("timeout"), Some(30));
        assert_eq!(job.get_config::<i32>("retries"), Some(3));
    }

    #[test]
    fn test_job_with_multiple_dependencies() {
        let dag = create_simple_dag();

        let deps = vec![
            JobDependency::new(JobId::new("job1")),
            JobDependency::new(JobId::new("job2")),
            JobDependency::new(JobId::new("job3")),
        ];

        let job = Job::new("multi_dep", "Multi Dep", dag).with_dependencies(deps);

        assert_eq!(job.dependencies().len(), 3);
    }

    #[test]
    fn test_job_debug_implementation() {
        let dag = create_simple_dag();
        let schedule = Schedule::new("0 * * * *").unwrap();

        let job = Job::new("debug_test", "Debug Test Job", dag)
            .with_schedule(schedule)
            .with_dependency(JobDependency::new(JobId::new("upstream")))
            .with_max_concurrency(2)
            .with_enabled(true);

        let debug_str = format!("{:?}", job);

        // Verify key fields are present in debug output
        assert!(debug_str.contains("debug_test"));
        assert!(debug_str.contains("Debug Test Job"));
        assert!(debug_str.contains("enabled: true"));
        assert!(debug_str.contains("test_dag")); // dag_id
        assert!(debug_str.contains("upstream")); // dependency
        assert!(debug_str.contains("max_concurrency: Some(2)"));
    }

    #[test]
    #[should_panic(expected = "max_concurrency cannot be zero")]
    fn test_job_with_max_concurrency_rejects_zero() {
        let dag = create_simple_dag();
        let _job =
            Job::new("zero_concurrency", "Zero Concurrency Job", dag).with_max_concurrency(0);
    }

    #[test]
    #[should_panic(expected = "max_concurrency cannot be zero")]
    fn test_job_builder_max_concurrency_rejects_zero() {
        let dag = create_simple_dag();
        let _job = JobBuilder::new("zero_concurrency", "Zero Concurrency Job")
            .dag(dag)
            .max_concurrency(0)
            .build()
            .unwrap();
    }

    #[test]
    fn test_job_builder_validates_zero_concurrency_in_build() {
        let dag = create_simple_dag();
        // Directly set max_concurrency to 0 (bypassing the setter)
        let mut builder = JobBuilder::new("zero_concurrency", "Zero Concurrency Job");
        builder.dag = Some(dag);
        builder.max_concurrency = Some(0);

        let result = builder.build();
        assert!(matches!(result, Err(JobError::InvalidConfig(_))));
        if let Err(JobError::InvalidConfig(msg)) = result {
            assert!(msg.contains("max_concurrency cannot be zero"));
        }
    }

    #[test]
    fn test_job_with_max_concurrency_accepts_positive_values() {
        let dag = create_simple_dag();
        let job1 = Job::new("job1", "Job 1", dag.clone()).with_max_concurrency(1);
        assert_eq!(job1.max_concurrency(), Some(1));

        let job2 = Job::new("job2", "Job 2", dag).with_max_concurrency(100);
        assert_eq!(job2.max_concurrency(), Some(100));
    }
}
