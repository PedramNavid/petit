//! Testing utilities for users of the Petit library.
//!
//! This module provides helpers for testing task orchestration:
//!
//! - [`MockTaskContext`]: A context that provides inputs and captures outputs
//! - [`TestHarness`]: Runs DAGs with in-memory storage
//! - [`FailingTask`]: A task helper that fails N times then succeeds

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::Mutex;

use crate::core::context::{ContextStore, TaskContext};
use crate::core::dag::Dag;
use crate::core::retry::RetryPolicy;
use crate::core::task::{Task, TaskError};
use crate::core::types::TaskId;
use crate::execution::{DagExecutor, DagResult};

/// A mock task context for testing.
///
/// Provides the ability to:
/// - Pre-set input values that tasks can read
/// - Capture outputs written by tasks
///
/// # Example
///
/// ```
/// use petit::testing::MockTaskContext;
///
/// let mut mock = MockTaskContext::new("my_task");
/// mock.set_input("upstream.value", 42);
///
/// // Run your task with mock.as_context()
/// let mut ctx = mock.as_context();
/// // After running a task, merge outputs back to the mock:
/// // mock.merge_outputs(&ctx);
/// // Then read the output:
/// // let output: i32 = mock.get_output("my_task.output").unwrap();
///
/// // Verify the input was set
/// let input: i32 = ctx.inputs.get("upstream.value").unwrap();
/// assert_eq!(input, 42);
/// ```
pub struct MockTaskContext {
    task_id: TaskId,
    store: ContextStore,
    data: Arc<RwLock<HashMap<String, Value>>>,
    config: Arc<HashMap<String, Value>>,
}

impl MockTaskContext {
    /// Create a new mock context for a task.
    pub fn new(task_id: impl Into<String>) -> Self {
        let data = Arc::new(RwLock::new(HashMap::new()));
        Self {
            task_id: TaskId::new(task_id),
            store: ContextStore::new(),
            data,
            config: Arc::new(HashMap::new()),
        }
    }

    /// Create a mock context with configuration values.
    pub fn with_config(task_id: impl Into<String>, config: HashMap<String, Value>) -> Self {
        let data = Arc::new(RwLock::new(HashMap::new()));
        Self {
            task_id: TaskId::new(task_id),
            store: ContextStore::new(),
            data,
            config: Arc::new(config),
        }
    }

    /// Set an input value that the task can read.
    ///
    /// Use the full namespaced key (e.g., "upstream_task.value").
    pub fn set_input<T: serde::Serialize>(&mut self, key: impl Into<String>, value: T) {
        let key = key.into();
        let json_value = serde_json::to_value(value).expect("failed to serialize input value");
        self.data
            .write()
            .expect("lock poisoned")
            .insert(key, json_value);
        // Rebuild the store with updated data
        self.store = ContextStore::from_map(self.data.read().expect("lock poisoned").clone());
    }

    /// Merge outputs from a TaskContext back into the mock.
    ///
    /// Call this after task execution to capture the outputs.
    pub fn merge_outputs(&mut self, ctx: &TaskContext) {
        let _ = self.store.merge(&ctx.outputs);
        // Update our data map to reflect the merged outputs
        if let Ok(mut data) = self.data.write() {
            for key in ctx.outputs.keys() {
                if let Some(value) = ctx.outputs.get_raw(&key) {
                    data.insert(key, value);
                }
            }
        }
    }

    /// Get an output value written by the task.
    ///
    /// Returns `None` if the key doesn't exist or deserialization fails.
    /// Note: Call `merge_outputs` first to ensure outputs are available.
    pub fn get_output<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.store.get_optional(key)
    }

    /// Check if an output key was written.
    pub fn has_output(&self, key: &str) -> bool {
        self.store.contains(key)
    }

    /// Get the underlying store as a HashMap.
    pub fn store(&self) -> HashMap<String, Value> {
        self.data.read().map(|s| s.clone()).unwrap_or_default()
    }

    /// Convert to a TaskContext for use in task execution.
    pub fn as_context(&self) -> TaskContext {
        TaskContext::new(
            self.store.clone(),
            self.task_id.clone(),
            self.config.clone(),
        )
    }
}

/// A task that fails a configurable number of times before succeeding.
///
/// Useful for testing retry logic and error handling.
///
/// This task is safe for concurrent execution - the failure counting is
/// protected by a mutex to ensure deterministic behavior.
///
/// # Example
///
/// ```
/// use petit::testing::FailingTask;
///
/// // Fails 2 times, then succeeds on the 3rd attempt
/// let task = FailingTask::new("flaky_task", 2);
/// ```
pub struct FailingTask {
    name: String,
    /// Mutex protecting failure state to prevent race conditions under concurrent execution.
    state: Mutex<FailingTaskState>,
    total_failures: u32,
    error_message: String,
    retry_policy: RetryPolicy,
}

/// Internal state for FailingTask, protected by a mutex.
struct FailingTaskState {
    failures_remaining: u32,
    call_count: u32,
}

impl FailingTask {
    /// Create a task that fails `fail_count` times then succeeds.
    pub fn new(name: impl Into<String>, fail_count: u32) -> Self {
        Self {
            name: name.into(),
            state: Mutex::new(FailingTaskState {
                failures_remaining: fail_count,
                call_count: 0,
            }),
            total_failures: fail_count,
            error_message: "intentional test failure".to_string(),
            retry_policy: RetryPolicy::none(),
        }
    }

    /// Create a task that fails with a custom error message.
    pub fn with_error(
        name: impl Into<String>,
        fail_count: u32,
        message: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            state: Mutex::new(FailingTaskState {
                failures_remaining: fail_count,
                call_count: 0,
            }),
            total_failures: fail_count,
            error_message: message.into(),
            retry_policy: RetryPolicy::none(),
        }
    }

    /// Set a retry policy for this task.
    pub fn with_retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }

    /// Get the number of failures remaining before success.
    ///
    /// Note: This is an async method because it acquires a lock.
    pub async fn failures_remaining(&self) -> u32 {
        self.state.lock().await.failures_remaining
    }

    /// Get the number of times this task has been called.
    ///
    /// Note: This is an async method because it acquires a lock.
    pub async fn call_count(&self) -> u32 {
        self.state.lock().await.call_count
    }

    /// Reset the failure counter for reuse.
    ///
    /// Note: This is an async method because it acquires a lock.
    pub async fn reset(&self) {
        let mut state = self.state.lock().await;
        state.failures_remaining = self.total_failures;
        state.call_count = 0;
    }
}

#[async_trait]
impl Task for FailingTask {
    fn name(&self) -> &str {
        &self.name
    }

    async fn execute(&self, ctx: &mut TaskContext) -> Result<(), TaskError> {
        // Lock the state to ensure atomic check-and-decrement under concurrent execution
        let mut state = self.state.lock().await;

        // Track call count
        state.call_count += 1;

        if state.failures_remaining > 0 {
            // Decrement and fail
            state.failures_remaining -= 1;
            Err(TaskError::ExecutionFailed(self.error_message.clone()))
        } else {
            // No failures remaining, succeed
            ctx.outputs.set("success", true)?;
            Ok(())
        }
    }

    fn retry_policy(&self) -> RetryPolicy {
        self.retry_policy.clone()
    }
}

/// Configuration for failure injection in TestHarness.
#[derive(Clone, Default)]
pub struct FailureInjection {
    /// Tasks that should always fail.
    pub always_fail: Vec<String>,
    /// Tasks that should fail with specific error messages.
    pub fail_with_message: HashMap<String, String>,
    /// Tasks that should fail N times then succeed.
    pub fail_n_times: HashMap<String, u32>,
}

impl FailureInjection {
    /// Create an empty failure injection config.
    pub fn new() -> Self {
        Self::default()
    }

    /// Configure a task to always fail.
    pub fn always_fail(mut self, task_id: impl Into<String>) -> Self {
        self.always_fail.push(task_id.into());
        self
    }

    /// Configure a task to fail with a specific message.
    pub fn fail_with(mut self, task_id: impl Into<String>, message: impl Into<String>) -> Self {
        self.fail_with_message
            .insert(task_id.into(), message.into());
        self
    }

    /// Configure a task to fail N times then succeed.
    pub fn fail_times(mut self, task_id: impl Into<String>, times: u32) -> Self {
        self.fail_n_times.insert(task_id.into(), times);
        self
    }
}

/// A test harness for running DAGs with in-memory storage.
///
/// Provides:
/// - Execution with configurable concurrency
/// - Failure injection configuration
/// - Assertions on execution results
///
/// # Example
///
/// ```ignore
/// use petit::testing::TestHarness;
/// use petit::DagBuilder;
///
/// let harness = TestHarness::new()
///     .with_concurrency(2);
///
/// let dag = DagBuilder::new("test", "Test DAG")
///     .add_task(my_task)
///     .build()?;
///
/// let result = harness.execute(&dag).await;
/// assert!(result.dag_result.success);
/// ```
pub struct TestHarness {
    executor: DagExecutor,
    failure_injection: FailureInjection,
    initial_context: HashMap<String, Value>,
}

/// Result from TestHarness execution.
pub struct TestResult {
    /// The DAG execution result.
    pub dag_result: DagResult,
    /// The final context after execution.
    pub context: HashMap<String, Value>,
}

impl TestHarness {
    /// Create a new test harness.
    pub fn new() -> Self {
        Self {
            executor: DagExecutor::with_concurrency(4),
            failure_injection: FailureInjection::default(),
            initial_context: HashMap::new(),
        }
    }

    /// Set the concurrency limit for task execution.
    pub fn with_concurrency(mut self, max_concurrency: usize) -> Self {
        self.executor = DagExecutor::with_concurrency(max_concurrency);
        self
    }

    /// Configure failure injection for specific tasks.
    pub fn with_failures(mut self, injection: FailureInjection) -> Self {
        self.failure_injection = injection;
        self
    }

    /// Set initial context values available to all tasks.
    pub fn with_context<T: serde::Serialize>(mut self, key: impl Into<String>, value: T) -> Self {
        let json_value = serde_json::to_value(value).expect("failed to serialize context value");
        self.initial_context.insert(key.into(), json_value);
        self
    }

    /// Execute a DAG and return the result.
    pub async fn execute(&self, dag: &Dag) -> TestResult {
        // Create context with initial values
        let store = ContextStore::from_map(self.initial_context.clone());
        let config = Arc::new(HashMap::new());
        let mut ctx = TaskContext::new(store.clone(), TaskId::new("harness"), config);

        // Execute the DAG
        let dag_result = self.executor.execute(dag, &mut ctx).await;

        // Extract final context (all keys from the store)
        let mut context = HashMap::new();
        for key in store.keys() {
            if let Some(value) = store.get_optional::<Value>(&key) {
                context.insert(key, value);
            }
        }

        TestResult {
            dag_result,
            context,
        }
    }

    /// Execute and assert success.
    pub async fn execute_and_assert_success(&self, dag: &Dag) -> TestResult {
        let result = self.execute(dag).await;
        assert!(
            result.dag_result.success,
            "Expected DAG to succeed, but it failed. Failed tasks: {:?}",
            result.dag_result.failed_tasks
        );
        result
    }

    /// Execute and assert failure.
    pub async fn execute_and_assert_failure(&self, dag: &Dag) -> TestResult {
        let result = self.execute(dag).await;
        assert!(
            !result.dag_result.success,
            "Expected DAG to fail, but it succeeded"
        );
        result
    }
}

impl Default for TestHarness {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::dag::DagBuilder;
    use crate::core::retry::RetryPolicy;
    use std::time::Duration;

    // Simple task for testing
    struct SimpleTask {
        name: String,
    }

    impl SimpleTask {
        fn new(name: &str) -> Arc<Self> {
            Arc::new(Self {
                name: name.to_string(),
            })
        }
    }

    #[async_trait]
    impl Task for SimpleTask {
        fn name(&self) -> &str {
            &self.name
        }

        async fn execute(&self, ctx: &mut TaskContext) -> Result<(), TaskError> {
            ctx.outputs.set("executed", true)?;
            Ok(())
        }
    }

    // Task that reads and transforms
    struct TransformTask {
        name: String,
        input_key: String,
    }

    impl TransformTask {
        fn new(name: &str, input_key: &str) -> Arc<Self> {
            Arc::new(Self {
                name: name.to_string(),
                input_key: input_key.to_string(),
            })
        }
    }

    #[async_trait]
    impl Task for TransformTask {
        fn name(&self) -> &str {
            &self.name
        }

        async fn execute(&self, ctx: &mut TaskContext) -> Result<(), TaskError> {
            let input: i32 = ctx.inputs.get(&self.input_key)?;
            ctx.outputs.set("result", input * 2)?;
            Ok(())
        }
    }

    // ==========================================================================
    // MockTaskContext Tests
    // ==========================================================================

    #[test]
    fn test_mock_context_provides_inputs() {
        let mut mock = MockTaskContext::new("test_task");
        mock.set_input("upstream.value", 42);
        mock.set_input("upstream.name", "hello");

        let ctx = mock.as_context();
        let value: i32 = ctx.inputs.get("upstream.value").unwrap();
        let name: String = ctx.inputs.get("upstream.name").unwrap();

        assert_eq!(value, 42);
        assert_eq!(name, "hello");
    }

    #[tokio::test]
    async fn test_mock_context_captures_outputs() {
        let mut mock = MockTaskContext::new("writer");
        let ctx = mock.as_context();

        // Simulate task writing output
        ctx.outputs.set("result", 100).unwrap();
        ctx.outputs.set("status", "done").unwrap();

        // Merge outputs back to mock
        mock.merge_outputs(&ctx);

        // Verify outputs through mock
        assert_eq!(mock.get_output::<i32>("writer.result"), Some(100));
        assert_eq!(
            mock.get_output::<String>("writer.status"),
            Some("done".to_string())
        );
    }

    #[test]
    fn test_mock_context_has_output() {
        let mut mock = MockTaskContext::new("task");
        mock.set_input("key", "value");

        assert!(mock.has_output("key"));
        assert!(!mock.has_output("nonexistent"));
    }

    #[test]
    fn test_mock_context_with_config() {
        let mut config = HashMap::new();
        config.insert("batch_size".to_string(), serde_json::json!(100));
        config.insert("timeout".to_string(), serde_json::json!(30));

        let mock = MockTaskContext::with_config("task", config);
        let ctx = mock.as_context();

        let batch_size: i32 = ctx.get_config("batch_size").unwrap();
        let timeout: i32 = ctx.get_config("timeout").unwrap();

        assert_eq!(batch_size, 100);
        assert_eq!(timeout, 30);
    }

    // ==========================================================================
    // FailingTask Tests
    // ==========================================================================

    #[tokio::test]
    async fn test_failing_task_fails_n_times_then_succeeds() {
        let task = FailingTask::new("flaky", 2);
        let mock = MockTaskContext::new("flaky");
        let mut ctx = mock.as_context();

        // First call - fails
        let result1 = task.execute(&mut ctx).await;
        assert!(result1.is_err());
        assert_eq!(task.call_count().await, 1);

        // Second call - fails
        let result2 = task.execute(&mut ctx).await;
        assert!(result2.is_err());
        assert_eq!(task.call_count().await, 2);

        // Third call - succeeds
        let result3 = task.execute(&mut ctx).await;
        assert!(result3.is_ok());
        assert_eq!(task.call_count().await, 3);
    }

    #[tokio::test]
    async fn test_failing_task_with_custom_error() {
        let task = FailingTask::with_error("bad_task", 1, "custom error message");
        let mock = MockTaskContext::new("bad_task");
        let mut ctx = mock.as_context();

        let result = task.execute(&mut ctx).await;
        let err = result.unwrap_err();

        assert!(err.to_string().contains("custom error message"));
    }

    #[tokio::test]
    async fn test_failing_task_reset() {
        let task = FailingTask::new("resettable", 1);
        let mock = MockTaskContext::new("resettable");
        let mut ctx = mock.as_context();

        // Fail once
        let _ = task.execute(&mut ctx).await;

        // Succeed
        let result = task.execute(&mut ctx).await;
        assert!(result.is_ok());

        // Reset and fail again
        task.reset().await;
        let result = task.execute(&mut ctx).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_failing_task_with_retry_policy() {
        let task = FailingTask::new("retry_task", 3)
            .with_retry_policy(RetryPolicy::fixed(5, Duration::from_millis(100)));

        let policy = task.retry_policy();
        assert_eq!(policy.max_attempts, 5);
    }

    // ==========================================================================
    // TestHarness Tests
    // ==========================================================================

    #[tokio::test]
    async fn test_harness_runs_dag_with_in_memory_storage() {
        let harness = TestHarness::new();

        let dag = DagBuilder::new("test", "Test DAG")
            .add_task(SimpleTask::new("task_a"))
            .build()
            .unwrap();

        let result = harness.execute(&dag).await;

        assert!(result.dag_result.success);
        assert_eq!(result.dag_result.completed_count(), 1);
    }

    #[tokio::test]
    async fn test_harness_execute_and_assert_success() {
        let harness = TestHarness::new();

        let dag = DagBuilder::new("test", "Test DAG")
            .add_task(SimpleTask::new("a"))
            .add_task_with_deps(SimpleTask::new("b"), &["a"])
            .build()
            .unwrap();

        let result = harness.execute_and_assert_success(&dag).await;
        assert_eq!(result.dag_result.completed_count(), 2);
    }

    #[tokio::test]
    async fn test_harness_execute_and_assert_failure() {
        let harness = TestHarness::new();

        let dag = DagBuilder::new("test", "Test DAG")
            .add_task(Arc::new(FailingTask::new("failing", 100)) as Arc<dyn Task>)
            .build()
            .unwrap();

        let result = harness.execute_and_assert_failure(&dag).await;
        assert_eq!(result.dag_result.failed_count(), 1);
    }

    #[tokio::test]
    async fn test_harness_with_initial_context() {
        let harness = TestHarness::new().with_context("initial.value", 21);

        let dag = DagBuilder::new("test", "Test DAG")
            .add_task(TransformTask::new("transform", "initial.value"))
            .build()
            .unwrap();

        let result = harness.execute(&dag).await;

        assert!(result.dag_result.success);
        let output: i32 =
            serde_json::from_value(result.context.get("transform.result").unwrap().clone())
                .unwrap();
        assert_eq!(output, 42);
    }

    // ==========================================================================
    // Failure Injection Tests
    // ==========================================================================

    #[tokio::test]
    async fn test_failure_injection_config() {
        let injection = FailureInjection::new()
            .always_fail("task_a")
            .fail_with("task_b", "custom error")
            .fail_times("task_c", 3);

        assert!(injection.always_fail.contains(&"task_a".to_string()));
        assert_eq!(
            injection.fail_with_message.get("task_b"),
            Some(&"custom error".to_string())
        );
        assert_eq!(injection.fail_n_times.get("task_c"), Some(&3));
    }

    #[tokio::test]
    async fn test_harness_with_failure_injection() {
        let harness =
            TestHarness::new().with_failures(FailureInjection::new().always_fail("will_fail"));

        // The failure injection is configured but the harness doesn't
        // automatically intercept tasks - this test verifies config works
        assert!(
            harness
                .failure_injection
                .always_fail
                .contains(&"will_fail".to_string())
        );
    }
}
