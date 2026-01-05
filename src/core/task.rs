//! Task trait and error types.
//!
//! The `Task` trait is the fundamental unit of work in the orchestrator.
//! Implement this trait to define custom tasks.

use async_trait::async_trait;
use thiserror::Error;

use super::context::TaskContext;
use super::environment::Environment;
use super::retry::RetryPolicy;

/// Errors that can occur during task execution.
#[derive(Debug, Error)]
pub enum TaskError {
    /// Task execution failed with a message.
    #[error("execution failed: {0}")]
    ExecutionFailed(String),

    /// Task timed out.
    #[error("task timed out after {0:?}")]
    Timeout(std::time::Duration),

    /// External command failed with exit code.
    #[error("command exited with code {code}: {stderr}")]
    CommandFailed {
        /// The exit code returned by the command.
        code: i32,
        /// The stderr output from the command.
        stderr: String,
    },

    /// Error accessing task context.
    #[error("context error: {0}")]
    Context(#[from] super::context::ContextError),

    /// A transient error that may succeed on retry.
    #[error("transient error: {0}")]
    Transient(String),

    /// Generic error wrapper.
    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

impl TaskError {
    /// Check if this error is considered transient (should trigger retry).
    pub fn is_transient(&self) -> bool {
        matches!(self, TaskError::Transient(_) | TaskError::Timeout(_))
    }
}

/// The core trait for defining executable tasks.
///
/// # Example
///
/// ```ignore
/// use petit::{Task, TaskContext, TaskError, Environment, RetryPolicy};
/// use async_trait::async_trait;
///
/// struct MyTask {
///     name: String,
/// }
///
/// #[async_trait]
/// impl Task for MyTask {
///     fn name(&self) -> &str {
///         &self.name
///     }
///
///     async fn execute(&self, ctx: &mut TaskContext) -> Result<(), TaskError> {
///         // Read input from upstream
///         let input: i32 = ctx.inputs.get("upstream.value")?;
///
///         // Do work
///         let result = input * 2;
///
///         // Write output for downstream
///         ctx.outputs.set("result", result)?;
///
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait Task: Send + Sync {
    /// Returns the unique name/identifier for this task type.
    fn name(&self) -> &str;

    /// Execute the task with the given context.
    ///
    /// # Arguments
    /// * `ctx` - Mutable context for reading inputs and writing outputs
    ///
    /// # Returns
    /// * `Ok(())` - Task completed successfully
    /// * `Err(TaskError)` - Task failed
    async fn execute(&self, ctx: &mut TaskContext) -> Result<(), TaskError>;

    /// Returns environment variables for this task.
    ///
    /// These will be merged with job-level environment and passed
    /// to the task during execution (e.g., to a subprocess).
    ///
    /// Default implementation returns an empty environment.
    fn environment(&self) -> Environment {
        Environment::default()
    }

    /// Returns the retry policy for this task.
    ///
    /// Default implementation returns no retries.
    fn retry_policy(&self) -> RetryPolicy {
        RetryPolicy::default()
    }

    /// Optional description for display/logging purposes.
    fn description(&self) -> Option<&str> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::ContextStore;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;

    // Helper to create a TaskContext for testing
    fn create_test_context(task_name: &str) -> TaskContext {
        let store = ContextStore::new();
        let config = Arc::new(HashMap::new());
        TaskContext::new(store, super::super::types::TaskId::new(task_name), config)
    }

    // A simple test task that succeeds
    struct SuccessTask {
        name: String,
    }

    #[async_trait]
    impl Task for SuccessTask {
        fn name(&self) -> &str {
            &self.name
        }

        async fn execute(&self, ctx: &mut TaskContext) -> Result<(), TaskError> {
            ctx.outputs.set("status", "completed")?;
            Ok(())
        }
    }

    // A task that always fails
    struct FailingTask {
        name: String,
        message: String,
    }

    #[async_trait]
    impl Task for FailingTask {
        fn name(&self) -> &str {
            &self.name
        }

        async fn execute(&self, _ctx: &mut TaskContext) -> Result<(), TaskError> {
            Err(TaskError::ExecutionFailed(self.message.clone()))
        }
    }

    // A task with custom environment
    struct TaskWithEnv {
        name: String,
    }

    #[async_trait]
    impl Task for TaskWithEnv {
        fn name(&self) -> &str {
            &self.name
        }

        async fn execute(&self, _ctx: &mut TaskContext) -> Result<(), TaskError> {
            Ok(())
        }

        fn environment(&self) -> Environment {
            Environment::new()
                .with_var("DATABASE_URL", "postgres://localhost/db")
                .with_var("API_KEY", "secret123")
        }
    }

    // A task with custom retry policy
    struct RetryableTask {
        name: String,
    }

    #[async_trait]
    impl Task for RetryableTask {
        fn name(&self) -> &str {
            &self.name
        }

        async fn execute(&self, _ctx: &mut TaskContext) -> Result<(), TaskError> {
            Ok(())
        }

        fn retry_policy(&self) -> RetryPolicy {
            RetryPolicy::fixed(3, Duration::from_secs(5))
        }
    }

    // A task that reads inputs and writes outputs
    struct TransformTask {
        name: String,
    }

    #[async_trait]
    impl Task for TransformTask {
        fn name(&self) -> &str {
            &self.name
        }

        async fn execute(&self, ctx: &mut TaskContext) -> Result<(), TaskError> {
            let input: i32 = ctx.inputs.get("upstream.value")?;
            let result = input * 2;
            ctx.outputs.set("result", result)?;
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_define_simple_task() {
        let task = SuccessTask {
            name: "my_task".to_string(),
        };

        assert_eq!(task.name(), "my_task");
    }

    #[tokio::test]
    async fn test_task_returns_success() {
        let task = SuccessTask {
            name: "success".to_string(),
        };
        let mut ctx = create_test_context("success");

        let result = task.execute(&mut ctx).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_task_writes_output() {
        let task = SuccessTask {
            name: "writer".to_string(),
        };
        let mut ctx = create_test_context("writer");

        task.execute(&mut ctx).await.unwrap();

        // Task writes to the local output buffer (prefixed with task id)
        let status: String =
            serde_json::from_value(ctx.outputs.get_raw("writer.status").unwrap()).unwrap();
        assert_eq!(status, "completed");
    }

    #[tokio::test]
    async fn test_task_returns_error() {
        let task = FailingTask {
            name: "failer".to_string(),
            message: "something went wrong".to_string(),
        };
        let mut ctx = create_test_context("failer");

        let result = task.execute(&mut ctx).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, TaskError::ExecutionFailed(_)));
        assert!(err.to_string().contains("something went wrong"));
    }

    #[tokio::test]
    async fn test_task_reads_inputs() {
        let task = TransformTask {
            name: "transform".to_string(),
        };

        // Set up context with upstream output
        let mut initial_data = HashMap::new();
        initial_data.insert("upstream.value".to_string(), serde_json::json!(21));
        let store = ContextStore::from_map(initial_data);
        let config = Arc::new(HashMap::new());
        let mut ctx = TaskContext::new(
            store.clone(),
            super::super::types::TaskId::new("transform"),
            config,
        );

        task.execute(&mut ctx).await.unwrap();

        // Merge outputs and verify
        store.merge(&ctx.outputs).unwrap();
        let result: i32 = store.get("transform.result").unwrap();
        assert_eq!(result, 42);
    }

    #[tokio::test]
    async fn test_default_environment() {
        let task = SuccessTask {
            name: "simple".to_string(),
        };

        let env = task.environment();

        assert!(env.is_empty());
    }

    #[tokio::test]
    async fn test_custom_environment() {
        let task = TaskWithEnv {
            name: "with_env".to_string(),
        };

        let env = task.environment();

        assert_eq!(env.get("DATABASE_URL"), Some("postgres://localhost/db"));
        assert_eq!(env.get("API_KEY"), Some("secret123"));
    }

    #[tokio::test]
    async fn test_default_retry_policy() {
        let task = SuccessTask {
            name: "simple".to_string(),
        };

        let policy = task.retry_policy();

        assert!(!policy.is_enabled());
        assert_eq!(policy.max_attempts, 0);
    }

    #[tokio::test]
    async fn test_custom_retry_policy() {
        let task = RetryableTask {
            name: "retryable".to_string(),
        };

        let policy = task.retry_policy();

        assert!(policy.is_enabled());
        assert_eq!(policy.max_attempts, 3);
        assert_eq!(policy.delay, Duration::from_secs(5));
    }

    #[test]
    fn test_task_error_is_transient() {
        let transient = TaskError::Transient("network timeout".to_string());
        let timeout = TaskError::Timeout(Duration::from_secs(30));
        let permanent = TaskError::ExecutionFailed("invalid input".to_string());

        assert!(transient.is_transient());
        assert!(timeout.is_transient());
        assert!(!permanent.is_transient());
    }

    #[test]
    fn test_task_error_display() {
        let err = TaskError::ExecutionFailed("test error".to_string());
        assert_eq!(err.to_string(), "execution failed: test error");

        let err = TaskError::CommandFailed {
            code: 1,
            stderr: "error message".to_string(),
        };
        assert_eq!(err.to_string(), "command exited with code 1: error message");
    }
}
