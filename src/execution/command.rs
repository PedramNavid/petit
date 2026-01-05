//! External command task implementation.
//!
//! [`CommandTask`] wraps shell commands and external executables, allowing them
//! to be used as tasks in a DAG. This module provides a builder pattern for
//! configuring commands with arguments, environment variables, timeouts, and
//! retry policies.
//!
//! # Quick Start
//!
//! ```rust
//! use petit::CommandTask;
//! use std::time::Duration;
//!
//! // Simple command
//! let task = CommandTask::builder("echo")
//!     .arg("hello")
//!     .build();
//!
//! // Command with timeout and retry
//! let robust_task = CommandTask::builder("curl")
//!     .args(["-s", "https://api.example.com/health"])
//!     .timeout(Duration::from_secs(30))
//!     .build();
//! ```
//!
//! # Multi-Stage Pipeline Example
//!
//! A typical ETL pipeline with extract, transform, and load stages:
//!
//! ```rust
//! use petit::{CommandTask, Environment, RetryPolicy};
//! use std::time::Duration;
//!
//! // Stage 1: Extract data from source
//! let extract = CommandTask::builder("python")
//!     .name("extract_data")
//!     .args(["-m", "etl.extract", "--source", "s3://bucket/raw"])
//!     .env("AWS_REGION", "us-east-1")
//!     .timeout(Duration::from_secs(300))
//!     .retry_policy(RetryPolicy::fixed(3, Duration::from_secs(10)))
//!     .build();
//!
//! // Stage 2: Transform data
//! let transform = CommandTask::builder("python")
//!     .name("transform_data")
//!     .args(["-m", "etl.transform", "--output", "/tmp/processed"])
//!     .working_dir("/app")
//!     .timeout(Duration::from_secs(600))
//!     .build();
//!
//! // Stage 3: Load to destination
//! // Note: env vars are passed as literal strings, not shell-expanded.
//! // To pass through a parent env var, read it explicitly:
//! let db_password = std::env::var("DB_PASSWORD").unwrap_or_default();
//! let load = CommandTask::builder("python")
//!     .name("load_data")
//!     .args(["-m", "etl.load", "--dest", "postgres://db/warehouse"])
//!     .env("DB_PASSWORD", db_password)
//!     .timeout(Duration::from_secs(300))
//!     .retry_policy(RetryPolicy::fixed(2, Duration::from_secs(30)))
//!     .build();
//! ```
//!
//! # Passing Data Between Tasks
//!
//! Tasks communicate through a shared context store. Each task can write outputs
//! that downstream tasks read as inputs. Outputs are automatically namespaced
//! by task name.
//!
//! When a [`CommandTask`] executes, its stdout and stderr are automatically
//! captured and stored in the context with keys `{task_name}.stdout` and
//! `{task_name}.stderr`.
//!
//! ```rust
//! use petit::{ContextStore, OutputBuffer, TaskId};
//! use std::collections::HashMap;
//!
//! // Create a shared context store (normally managed by the executor)
//! let store = ContextStore::new();
//!
//! // Upstream task writes output (simulating what CommandTask does internally)
//! let buffer = OutputBuffer::new(TaskId::new("extract"));
//! buffer.set("row_count", 1000).unwrap();
//! buffer.set("status", "success").unwrap();
//!
//! // Merge outputs to shared store (executor does this on success)
//! store.merge(&buffer).unwrap();
//!
//! // Downstream task reads the data using "{upstream_task}.{key}" format
//! let count: i32 = store.get("extract.row_count").unwrap();
//! let status: String = store.get("extract.status").unwrap();
//! assert_eq!(count, 1000);
//! assert_eq!(status, "success");
//! ```
//!
//! # Timeout Handling Patterns
//!
//! Timeouts prevent tasks from running indefinitely. When a timeout occurs,
//! the task returns [`TaskError::Timeout`] which is considered a transient
//! error and can trigger retries.
//!
//! **Process Termination**: When a timeout occurs, the subprocess is terminated
//! when the underlying command future is dropped. Note that this may not give
//! the process time for graceful shutdown. If your command requires cleanup,
//! consider implementing signal handling within the command itself or using
//! shorter timeouts with retry policies.
//!
//! ```rust
//! use petit::{CommandTask, RetryPolicy, RetryCondition};
//! use std::time::Duration;
//!
//! // Basic timeout - task fails if not complete within 30 seconds
//! let quick_task = CommandTask::builder("./check_health.sh")
//!     .timeout(Duration::from_secs(30))
//!     .build();
//!
//! // Timeout with retry - retries on timeout up to 3 times
//! let resilient_task = CommandTask::builder("./process_batch.sh")
//!     .timeout(Duration::from_secs(60))
//!     .retry_policy(
//!         RetryPolicy::fixed(3, Duration::from_secs(5))
//!             .with_condition(RetryCondition::TransientOnly)
//!     )
//!     .build();
//!
//! // Long-running task with generous timeout
//! let batch_job = CommandTask::builder("python")
//!     .name("nightly_batch")
//!     .args(["-m", "batch.process", "--full"])
//!     .timeout(Duration::from_secs(3600))  // 1 hour
//!     .build();
//! ```
//!
//! # Environment Variable Patterns
//!
//! Environment variables can be set at multiple levels and are passed to
//! the subprocess when the command executes.
//!
//! ```rust
//! use petit::{CommandTask, Environment};
//!
//! // Single environment variable using fluent builder
//! let task = CommandTask::builder("./deploy.sh")
//!     .env("ENVIRONMENT", "production")
//!     .env("LOG_LEVEL", "info")
//!     .build();
//!
//! // Multiple variables from an Environment object
//! let env = Environment::new()
//!     .with_var("DATABASE_URL", "postgres://localhost/mydb")
//!     .with_var("REDIS_URL", "redis://localhost:6379")
//!     .with_var("API_KEY", "secret123");
//!
//! let task_with_env = CommandTask::builder("python")
//!     .args(["-m", "app.main"])
//!     .environment(env)
//!     .build();
//!
//! // Combining job-level and task-level environment variables
//! // In YAML configuration, variables merge with task-level taking precedence:
//! // ```yaml
//! // environment:  # Job-level
//! //   LOG_LEVEL: info
//! // tasks:
//! //   - id: my_task
//! //     environment:  # Task-level (overrides job-level)
//! //       LOG_LEVEL: debug
//! // ```
//! ```
//!
//! # Error Handling
//!
//! [`CommandTask`] can fail in several ways:
//!
//! - **Non-zero exit code**: Returns [`TaskError::CommandFailed`] with the exit
//!   code and stderr output
//! - **Timeout**: Returns [`TaskError::Timeout`] (transient, can retry)
//! - **Execution failure**: Returns [`TaskError::ExecutionFailed`] if the
//!   command cannot be started (e.g., program not found)
//!
//! ```rust
//! use petit::TaskError;
//! use std::time::Duration;
//!
//! // Example: checking error types for appropriate handling
//! fn handle_task_error(err: TaskError) {
//!     match err {
//!         TaskError::CommandFailed { code, stderr } => {
//!             eprintln!("Command failed with exit code {}: {}", code, stderr);
//!         }
//!         TaskError::Timeout(duration) => {
//!             eprintln!("Command timed out after {:?}", duration);
//!             // Timeout errors are transient - retry may succeed
//!         }
//!         TaskError::ExecutionFailed(msg) => {
//!             eprintln!("Failed to execute: {}", msg);
//!         }
//!         _ => eprintln!("Other error: {}", err),
//!     }
//! }
//!
//! // Check if an error is transient (suitable for retry)
//! let timeout_err = TaskError::Timeout(Duration::from_secs(30));
//! assert!(timeout_err.is_transient());
//!
//! let cmd_err = TaskError::CommandFailed { code: 1, stderr: "error".into() };
//! assert!(!cmd_err.is_transient());
//! ```

use async_trait::async_trait;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

use crate::core::context::TaskContext;
use crate::core::environment::Environment;
use crate::core::retry::RetryPolicy;
use crate::core::task::{Task, TaskError};

/// A task that executes an external command.
///
/// # Example
///
/// ```ignore
/// let task = CommandTask::builder("echo")
///     .arg("hello")
///     .arg("world")
///     .environment(Environment::new().with_var("MY_VAR", "value"))
///     .working_dir("/tmp")
///     .build();
/// ```
#[derive(Debug, Clone)]
pub struct CommandTask {
    /// Task name (used for identification)
    name: String,
    /// Program to execute
    program: String,
    /// Command arguments
    args: Vec<String>,
    /// Environment variables
    environment: Environment,
    /// Working directory
    working_dir: Option<PathBuf>,
    /// Retry policy
    retry_policy: RetryPolicy,
    /// Execution timeout
    timeout: Option<Duration>,
}

impl CommandTask {
    /// Create a new builder for a command task.
    pub fn builder(program: impl Into<String>) -> CommandTaskBuilder {
        CommandTaskBuilder::new(program)
    }

    /// Get the program being executed.
    pub fn program(&self) -> &str {
        &self.program
    }

    /// Get the command arguments.
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Get the working directory.
    pub fn working_dir(&self) -> Option<&PathBuf> {
        self.working_dir.as_ref()
    }

    /// Get the timeout duration.
    pub fn timeout(&self) -> Option<Duration> {
        self.timeout
    }
}

#[async_trait]
impl Task for CommandTask {
    fn name(&self) -> &str {
        &self.name
    }

    async fn execute(&self, ctx: &mut TaskContext) -> Result<(), TaskError> {
        let mut cmd = Command::new(&self.program);

        // Add arguments
        cmd.args(&self.args);

        // Set environment variables
        for (key, value) in self.environment.iter() {
            cmd.env(key, value);
        }

        // Set working directory if specified
        if let Some(ref dir) = self.working_dir {
            cmd.current_dir(dir);
        }

        // Capture stdout and stderr
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // Execute with optional timeout
        let output = match self.timeout {
            Some(duration) => timeout(duration, cmd.output())
                .await
                .map_err(|_| TaskError::Timeout(duration))?
                .map_err(|e| TaskError::ExecutionFailed(e.to_string()))?,
            None => cmd
                .output()
                .await
                .map_err(|e| TaskError::ExecutionFailed(e.to_string()))?,
        };

        // Store stdout and stderr in context (always write, even if empty,
        // so downstream tasks can distinguish "not run" from "no output")
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        ctx.outputs
            .set("stdout", &stdout)
            .map_err(|e| TaskError::ExecutionFailed(e.to_string()))?;
        ctx.outputs
            .set("stderr", &stderr)
            .map_err(|e| TaskError::ExecutionFailed(e.to_string()))?;

        // Store exit code in context (always, so downstream tasks can check it)
        let code = output.status.code().unwrap_or(-1);
        ctx.outputs
            .set("exit_code", code)
            .map_err(|e| TaskError::ExecutionFailed(e.to_string()))?;

        // Check exit status
        if output.status.success() {
            Ok(())
        } else {
            Err(TaskError::CommandFailed { code, stderr })
        }
    }

    fn environment(&self) -> Environment {
        self.environment.clone()
    }

    fn retry_policy(&self) -> RetryPolicy {
        self.retry_policy.clone()
    }
}

/// Builder for creating `CommandTask` instances.
#[derive(Debug, Clone)]
pub struct CommandTaskBuilder {
    name: Option<String>,
    program: String,
    args: Vec<String>,
    environment: Environment,
    working_dir: Option<PathBuf>,
    retry_policy: RetryPolicy,
    timeout: Option<Duration>,
}

impl CommandTaskBuilder {
    /// Create a new builder with the given program.
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            name: None,
            program: program.into(),
            args: Vec::new(),
            environment: Environment::default(),
            working_dir: None,
            retry_policy: RetryPolicy::default(),
            timeout: None,
        }
    }

    /// Set the task name.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Add a single argument.
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Add multiple arguments.
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Set environment variables.
    pub fn environment(mut self, env: Environment) -> Self {
        self.environment = env;
        self
    }

    /// Add a single environment variable.
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.environment = self.environment.with_var(key, value);
        self
    }

    /// Set the working directory.
    pub fn working_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    /// Set the retry policy.
    pub fn retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }

    /// Set the execution timeout.
    pub fn timeout(mut self, duration: Duration) -> Self {
        self.timeout = Some(duration);
        self
    }

    /// Build the `CommandTask`.
    pub fn build(self) -> CommandTask {
        let name = self.name.unwrap_or_else(|| self.program.clone());
        CommandTask {
            name,
            program: self.program,
            args: self.args,
            environment: self.environment,
            working_dir: self.working_dir,
            retry_policy: self.retry_policy,
            timeout: self.timeout,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::ContextStore;
    use crate::core::retry::RetryCondition;
    use crate::core::types::TaskId;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn create_test_context() -> TaskContext {
        let store = ContextStore::new();
        let config = Arc::new(HashMap::new());
        TaskContext::new(store, TaskId::new("test"), config)
    }

    /// Helper to get output from context buffer after task execution
    fn get_output<T: serde::de::DeserializeOwned>(ctx: &TaskContext, key: &str) -> Option<T> {
        ctx.outputs
            .get_raw(key)
            .and_then(|v| serde_json::from_value(v).ok())
    }

    #[test]
    fn test_create_command_task_with_program_and_args() {
        let task = CommandTask::builder("echo")
            .arg("hello")
            .arg("world")
            .build();

        assert_eq!(task.program(), "echo");
        assert_eq!(task.args(), &["hello", "world"]);
    }

    #[test]
    fn test_command_task_implements_task_trait() {
        let task = CommandTask::builder("echo").build();

        // Task trait methods should work
        assert_eq!(task.name(), "echo");
        assert_eq!(task.retry_policy(), RetryPolicy::default());
        assert_eq!(task.environment(), Environment::default());
    }

    #[test]
    fn test_command_task_with_custom_name() {
        let task = CommandTask::builder("python")
            .name("run_script")
            .arg("script.py")
            .build();

        assert_eq!(task.name(), "run_script");
        assert_eq!(task.program(), "python");
    }

    #[tokio::test]
    async fn test_execute_simple_command() {
        let task = CommandTask::builder("echo")
            .name("test")
            .arg("hello")
            .build();

        let mut ctx = create_test_context();
        let result = task.execute(&mut ctx).await;

        assert!(result.is_ok());

        // Check stdout was captured (stored with task name prefix)
        let stdout: String = get_output(&ctx, "test.stdout").unwrap();
        assert_eq!(stdout.trim(), "hello");
    }

    #[tokio::test]
    async fn test_command_with_environment_variables() {
        let task = CommandTask::builder("sh")
            .name("test")
            .arg("-c")
            .arg("echo $MY_VAR")
            .env("MY_VAR", "test_value")
            .build();

        let mut ctx = create_test_context();
        let result = task.execute(&mut ctx).await;

        assert!(result.is_ok());

        let stdout: String = get_output(&ctx, "test.stdout").unwrap();
        assert_eq!(stdout.trim(), "test_value");
    }

    #[tokio::test]
    async fn test_command_with_working_directory() {
        let task = CommandTask::builder("pwd")
            .name("test")
            .working_dir("/tmp")
            .build();

        let mut ctx = create_test_context();
        let result = task.execute(&mut ctx).await;

        assert!(result.is_ok());

        let stdout: String = get_output(&ctx, "test.stdout").unwrap();
        assert_eq!(stdout.trim(), "/tmp");
    }

    #[tokio::test]
    async fn test_command_returns_exit_code_on_failure() {
        let task = CommandTask::builder("sh").arg("-c").arg("exit 42").build();

        let mut ctx = create_test_context();
        let result = task.execute(&mut ctx).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            TaskError::CommandFailed { code, .. } => {
                assert_eq!(code, 42);
            }
            other => panic!("Expected CommandFailed, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_command_stores_exit_code_on_success() {
        let task = CommandTask::builder("true").name("test").build();

        let mut ctx = create_test_context();
        let result = task.execute(&mut ctx).await;

        assert!(result.is_ok());

        // Exit code should be stored even on success
        let exit_code: i32 = get_output(&ctx, "test.exit_code").unwrap();
        assert_eq!(exit_code, 0);
    }

    #[tokio::test]
    async fn test_command_stores_exit_code_on_failure() {
        let task = CommandTask::builder("sh")
            .name("test")
            .arg("-c")
            .arg("exit 42")
            .build();

        let mut ctx = create_test_context();
        let result = task.execute(&mut ctx).await;

        assert!(result.is_err());

        // Exit code should be stored even on failure
        let exit_code: i32 = get_output(&ctx, "test.exit_code").unwrap();
        assert_eq!(exit_code, 42);
    }

    #[tokio::test]
    async fn test_command_captures_stdout_and_stderr() {
        let task = CommandTask::builder("sh")
            .name("test")
            .arg("-c")
            .arg("echo stdout_msg; echo stderr_msg >&2")
            .build();

        let mut ctx = create_test_context();
        let result = task.execute(&mut ctx).await;

        assert!(result.is_ok());

        let stdout: String = get_output(&ctx, "test.stdout").unwrap();
        let stderr: String = get_output(&ctx, "test.stderr").unwrap();

        assert_eq!(stdout.trim(), "stdout_msg");
        assert_eq!(stderr.trim(), "stderr_msg");
    }

    #[tokio::test]
    async fn test_command_with_timeout() {
        let task = CommandTask::builder("sleep")
            .arg("10")
            .timeout(Duration::from_millis(100))
            .build();

        let mut ctx = create_test_context();
        let result = task.execute(&mut ctx).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            TaskError::Timeout(duration) => {
                assert_eq!(duration, Duration::from_millis(100));
            }
            other => panic!("Expected Timeout, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_command_writes_empty_stdout_stderr_to_context() {
        // Command that produces no output
        let task = CommandTask::builder("true").name("test").build();

        let mut ctx = create_test_context();
        let result = task.execute(&mut ctx).await;

        assert!(result.is_ok());

        // Even with no output, stdout and stderr should be written to context
        // so downstream tasks can distinguish "not run" from "no output"
        let stdout: String = get_output(&ctx, "test.stdout").unwrap();
        let stderr: String = get_output(&ctx, "test.stderr").unwrap();

        assert_eq!(stdout, "");
        assert_eq!(stderr, "");
    }

    #[test]
    fn test_command_with_multiple_args() {
        let task = CommandTask::builder("echo")
            .args(["one", "two", "three"])
            .build();

        assert_eq!(task.args(), &["one", "two", "three"]);
    }

    #[test]
    fn test_command_with_environment_from_builder() {
        let env = Environment::new()
            .with_var("KEY1", "value1")
            .with_var("KEY2", "value2");

        let task = CommandTask::builder("echo")
            .environment(env.clone())
            .build();

        assert_eq!(task.environment(), env);
    }

    #[test]
    fn test_command_with_retry_policy() {
        let policy =
            RetryPolicy::fixed(3, Duration::from_secs(5)).with_condition(RetryCondition::Always);

        let task = CommandTask::builder("echo")
            .retry_policy(policy.clone())
            .build();

        assert_eq!(task.retry_policy(), policy);
    }

    #[test]
    fn test_builder_chaining() {
        let task = CommandTask::builder("python")
            .name("my_task")
            .arg("-c")
            .arg("print('hello')")
            .env("PYTHONPATH", "/app")
            .working_dir("/tmp")
            .timeout(Duration::from_secs(30))
            .retry_policy(RetryPolicy::fixed(2, Duration::from_secs(1)))
            .build();

        assert_eq!(task.name(), "my_task");
        assert_eq!(task.program(), "python");
        assert_eq!(task.args(), &["-c", "print('hello')"]);
        assert_eq!(task.working_dir(), Some(&PathBuf::from("/tmp")));
        assert_eq!(task.timeout(), Some(Duration::from_secs(30)));
    }

    #[tokio::test]
    async fn test_long_running_command_timeout_completes_quickly() {
        // This test verifies that when a timeout occurs on a long-running command,
        // we return promptly rather than waiting for the command to complete.
        // This ensures the timeout mechanism works correctly for commands that
        // would otherwise run much longer than the timeout duration.
        let task = CommandTask::builder("sleep")
            .arg("60") // Would take 60 seconds without timeout
            .timeout(Duration::from_millis(200))
            .build();

        let mut ctx = create_test_context();
        let start = std::time::Instant::now();
        let result = task.execute(&mut ctx).await;
        let elapsed = start.elapsed();

        // Should have timed out
        assert!(result.is_err());
        match result.unwrap_err() {
            TaskError::Timeout(duration) => {
                assert_eq!(duration, Duration::from_millis(200));
            }
            other => panic!("Expected Timeout, got {:?}", other),
        }

        // Should have completed quickly (timeout + some margin), not 60 seconds
        assert!(
            elapsed < Duration::from_secs(1),
            "Timeout took too long: {:?}. Expected ~200ms, got {:?}",
            elapsed,
            elapsed
        );
    }

    #[tokio::test]
    async fn test_timeout_returns_correct_error_type() {
        // Verify that timeout errors are marked as transient (retriable)
        let task = CommandTask::builder("sleep")
            .arg("60")
            .timeout(Duration::from_millis(50))
            .build();

        let mut ctx = create_test_context();
        let result = task.execute(&mut ctx).await;

        let err = result.unwrap_err();
        assert!(
            err.is_transient(),
            "Timeout errors should be transient for retry purposes"
        );
    }
}
