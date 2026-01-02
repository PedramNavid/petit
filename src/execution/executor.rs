//! Task execution engine.
//!
//! The `TaskExecutor` handles running individual tasks with:
//! - Retry logic based on task's retry policy
//! - Concurrency limiting via semaphore
//! - Proper error handling and result reporting

use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Semaphore;
use tokio::time::sleep;

use crate::core::context::TaskContext;
use crate::core::retry::RetryCondition;
use crate::core::task::Task;
use crate::core::types::TaskId;

/// Result of executing a task.
#[derive(Debug, Clone)]
pub struct TaskResult {
    /// The task that was executed.
    pub task_id: TaskId,
    /// Whether the task succeeded.
    pub success: bool,
    /// Number of attempts made (1 = first try, 2+ = retries).
    pub attempts: u32,
    /// Total duration of all attempts.
    pub duration: std::time::Duration,
    /// Error if the task failed.
    pub error: Option<String>,
}

impl TaskResult {
    /// Create a successful result.
    pub fn success(task_id: TaskId, attempts: u32, duration: std::time::Duration) -> Self {
        Self {
            task_id,
            success: true,
            attempts,
            duration,
            error: None,
        }
    }

    /// Create a failed result.
    pub fn failure(
        task_id: TaskId,
        attempts: u32,
        duration: std::time::Duration,
        error: String,
    ) -> Self {
        Self {
            task_id,
            success: false,
            attempts,
            duration,
            error: Some(error),
        }
    }
}

/// Executor for running tasks with concurrency control and retry logic.
pub struct TaskExecutor {
    /// Maximum number of concurrent task executions.
    max_concurrency: usize,
    /// Semaphore for concurrency control.
    semaphore: Arc<Semaphore>,
}

impl TaskExecutor {
    /// Create a new executor with the given concurrency limit.
    pub fn new(max_concurrency: usize) -> Self {
        Self {
            max_concurrency,
            semaphore: Arc::new(Semaphore::new(max_concurrency)),
        }
    }

    /// Get the maximum concurrency limit.
    pub fn max_concurrency(&self) -> usize {
        self.max_concurrency
    }

    /// Get the number of available permits (slots for concurrent execution).
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// Execute a task with retry logic.
    ///
    /// This method will:
    /// 1. Acquire a concurrency permit
    /// 2. Execute the task
    /// 3. Retry on failure according to the task's retry policy
    /// 4. Return the result with attempt count and duration
    pub async fn execute(
        &self,
        task: &dyn Task,
        ctx: &mut TaskContext,
    ) -> TaskResult {
        let task_id = TaskId::new(task.name());
        let start_time = Instant::now();
        let retry_policy = task.retry_policy();

        // Acquire semaphore permit for concurrency control
        let _permit = self.semaphore.acquire().await.expect("semaphore closed");

        let mut attempts = 0u32;

        // First attempt + retries
        let max_attempts = retry_policy.max_attempts + 1; // +1 for initial attempt

        loop {
            attempts += 1;

            match task.execute(ctx).await {
                Ok(()) => {
                    return TaskResult::success(task_id, attempts, start_time.elapsed());
                }
                Err(err) => {
                    // Check if we should retry
                    let should_retry = attempts < max_attempts && match retry_policy.retry_on {
                        RetryCondition::Always => true,
                        RetryCondition::TransientOnly => err.is_transient(),
                        RetryCondition::Never => false,
                    };

                    if should_retry {
                        // Wait before retrying
                        sleep(retry_policy.delay).await;
                    } else {
                        // No more retries, return failure
                        return TaskResult::failure(
                            task_id,
                            attempts,
                            start_time.elapsed(),
                            err.to_string(),
                        );
                    }
                }
            }
        }
    }

    /// Execute a task without acquiring a semaphore permit.
    ///
    /// Use this when you're managing concurrency externally or for testing.
    pub async fn execute_without_permit(
        &self,
        task: &dyn Task,
        ctx: &mut TaskContext,
    ) -> TaskResult {
        let task_id = TaskId::new(task.name());
        let start_time = Instant::now();
        let retry_policy = task.retry_policy();

        let mut attempts = 0u32;
        let max_attempts = retry_policy.max_attempts + 1;

        loop {
            attempts += 1;

            match task.execute(ctx).await {
                Ok(()) => {
                    return TaskResult::success(task_id, attempts, start_time.elapsed());
                }
                Err(err) => {
                    let should_retry = attempts < max_attempts && match retry_policy.retry_on {
                        RetryCondition::Always => true,
                        RetryCondition::TransientOnly => err.is_transient(),
                        RetryCondition::Never => false,
                    };

                    if should_retry {
                        sleep(retry_policy.delay).await;
                    } else {
                        return TaskResult::failure(
                            task_id,
                            attempts,
                            start_time.elapsed(),
                            err.to_string(),
                        );
                    }
                }
            }
        }
    }
}

impl Default for TaskExecutor {
    fn default() -> Self {
        Self::new(4) // Default to 4 concurrent tasks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::TaskContext;
    use crate::core::retry::RetryPolicy;
    use crate::core::task::TaskError;
    use crate::core::types::TaskId;
    use async_trait::async_trait;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, RwLock};
    use std::time::Duration;

    fn create_test_context() -> TaskContext {
        let store = Arc::new(RwLock::new(HashMap::<String, Value>::new()));
        let config = Arc::new(HashMap::new());
        TaskContext::new(store, TaskId::new("test"), config)
    }

    // A simple task that always succeeds
    struct SuccessTask {
        name: String,
    }

    #[async_trait]
    impl Task for SuccessTask {
        fn name(&self) -> &str {
            &self.name
        }

        async fn execute(&self, ctx: &mut TaskContext) -> Result<(), TaskError> {
            ctx.outputs.set("status", "done")?;
            Ok(())
        }
    }

    // A task that always fails
    struct FailingTask {
        name: String,
        retry_policy: RetryPolicy,
    }

    #[async_trait]
    impl Task for FailingTask {
        fn name(&self) -> &str {
            &self.name
        }

        async fn execute(&self, _ctx: &mut TaskContext) -> Result<(), TaskError> {
            Err(TaskError::ExecutionFailed("always fails".to_string()))
        }

        fn retry_policy(&self) -> RetryPolicy {
            self.retry_policy.clone()
        }
    }

    // A task that fails N times then succeeds
    struct EventuallySucceedsTask {
        name: String,
        failures_remaining: Arc<AtomicU32>,
        retry_policy: RetryPolicy,
    }

    impl EventuallySucceedsTask {
        fn new(name: &str, fail_count: u32, retry_policy: RetryPolicy) -> Self {
            Self {
                name: name.to_string(),
                failures_remaining: Arc::new(AtomicU32::new(fail_count)),
                retry_policy,
            }
        }
    }

    #[async_trait]
    impl Task for EventuallySucceedsTask {
        fn name(&self) -> &str {
            &self.name
        }

        async fn execute(&self, _ctx: &mut TaskContext) -> Result<(), TaskError> {
            let remaining = self.failures_remaining.fetch_sub(1, Ordering::SeqCst);
            if remaining > 0 {
                Err(TaskError::ExecutionFailed(format!(
                    "failing, {} more to go",
                    remaining - 1
                )))
            } else {
                Ok(())
            }
        }

        fn retry_policy(&self) -> RetryPolicy {
            self.retry_policy.clone()
        }
    }

    // A task that tracks how many times it was called
    struct CountingTask {
        name: String,
        call_count: Arc<AtomicU32>,
        retry_policy: RetryPolicy,
    }

    impl CountingTask {
        fn new(name: &str, retry_policy: RetryPolicy) -> Self {
            Self {
                name: name.to_string(),
                call_count: Arc::new(AtomicU32::new(0)),
                retry_policy,
            }
        }

        fn count(&self) -> u32 {
            self.call_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl Task for CountingTask {
        fn name(&self) -> &str {
            &self.name
        }

        async fn execute(&self, _ctx: &mut TaskContext) -> Result<(), TaskError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            Err(TaskError::ExecutionFailed("always fails".to_string()))
        }

        fn retry_policy(&self) -> RetryPolicy {
            self.retry_policy.clone()
        }
    }

    // A slow task for testing concurrency
    struct SlowTask {
        name: String,
        duration: Duration,
    }

    #[async_trait]
    impl Task for SlowTask {
        fn name(&self) -> &str {
            &self.name
        }

        async fn execute(&self, _ctx: &mut TaskContext) -> Result<(), TaskError> {
            tokio::time::sleep(self.duration).await;
            Ok(())
        }
    }

    // A task that fails with transient errors
    struct TransientFailTask {
        name: String,
        failures_remaining: Arc<AtomicU32>,
        retry_policy: RetryPolicy,
    }

    impl TransientFailTask {
        fn new(name: &str, fail_count: u32, retry_policy: RetryPolicy) -> Self {
            Self {
                name: name.to_string(),
                failures_remaining: Arc::new(AtomicU32::new(fail_count)),
                retry_policy,
            }
        }
    }

    #[async_trait]
    impl Task for TransientFailTask {
        fn name(&self) -> &str {
            &self.name
        }

        async fn execute(&self, _ctx: &mut TaskContext) -> Result<(), TaskError> {
            let remaining = self.failures_remaining.fetch_sub(1, Ordering::SeqCst);
            if remaining > 0 {
                Err(TaskError::Transient("temporary failure".to_string()))
            } else {
                Ok(())
            }
        }

        fn retry_policy(&self) -> RetryPolicy {
            self.retry_policy.clone()
        }
    }

    #[tokio::test]
    async fn test_execute_single_task_successfully() {
        let executor = TaskExecutor::new(4);
        let task = SuccessTask {
            name: "success_task".to_string(),
        };
        let mut ctx = create_test_context();

        let result = executor.execute(&task, &mut ctx).await;

        assert!(result.success);
        assert_eq!(result.attempts, 1);
        assert!(result.error.is_none());
    }

    #[tokio::test]
    async fn test_execute_task_that_fails() {
        let executor = TaskExecutor::new(4);
        let task = FailingTask {
            name: "failing_task".to_string(),
            retry_policy: RetryPolicy::none(),
        };
        let mut ctx = create_test_context();

        let result = executor.execute(&task, &mut ctx).await;

        assert!(!result.success);
        assert_eq!(result.attempts, 1);
        assert!(result.error.is_some());
        assert!(result.error.unwrap().contains("always fails"));
    }

    #[tokio::test]
    async fn test_execute_task_with_retry_on_failure() {
        let executor = TaskExecutor::new(4);
        // Fails twice, then succeeds. With 3 max_attempts, should succeed on 3rd try.
        let task = EventuallySucceedsTask::new(
            "eventually_succeeds",
            2, // fail 2 times
            RetryPolicy::fixed(3, Duration::from_millis(10)),
        );
        let mut ctx = create_test_context();

        let result = executor.execute(&task, &mut ctx).await;

        assert!(result.success);
        assert_eq!(result.attempts, 3); // 2 failures + 1 success
    }

    #[tokio::test]
    async fn test_retry_respects_fixed_delay() {
        let executor = TaskExecutor::new(4);
        let delay = Duration::from_millis(50);
        let task = EventuallySucceedsTask::new(
            "delayed_retry",
            1, // fail once
            RetryPolicy::fixed(2, delay),
        );
        let mut ctx = create_test_context();

        let start = Instant::now();
        let result = executor.execute(&task, &mut ctx).await;
        let elapsed = start.elapsed();

        assert!(result.success);
        assert_eq!(result.attempts, 2);
        // Should have waited at least the delay duration
        assert!(elapsed >= delay, "Expected at least {:?}, got {:?}", delay, elapsed);
    }

    #[tokio::test]
    async fn test_retry_stops_after_max_attempts() {
        let executor = TaskExecutor::new(4);
        let task = CountingTask::new(
            "counting_task",
            RetryPolicy::fixed(2, Duration::from_millis(1)), // max 2 retries = 3 total attempts
        );
        let mut ctx = create_test_context();

        let result = executor.execute(&task, &mut ctx).await;

        assert!(!result.success);
        assert_eq!(result.attempts, 3); // 1 initial + 2 retries
        assert_eq!(task.count(), 3);
    }

    #[tokio::test]
    async fn test_concurrent_execution_respects_limit() {
        let executor = Arc::new(TaskExecutor::new(2)); // Only 2 concurrent
        let task_duration = Duration::from_millis(50);

        // Launch 4 tasks that each take 50ms
        let mut handles = vec![];
        for i in 0..4 {
            let executor = Arc::clone(&executor);
            let task = SlowTask {
                name: format!("slow_task_{}", i),
                duration: task_duration,
            };
            handles.push(tokio::spawn(async move {
                let mut ctx = create_test_context();
                executor.execute(&task, &mut ctx).await
            }));
        }

        let start = Instant::now();
        for handle in handles {
            handle.await.unwrap();
        }
        let elapsed = start.elapsed();

        // With 2 concurrent slots and 4 tasks of 50ms each:
        // First 2 run in parallel (50ms), then next 2 run in parallel (50ms)
        // Total should be ~100ms, not ~200ms (if sequential) or ~50ms (if unlimited)
        assert!(
            elapsed >= Duration::from_millis(90) && elapsed < Duration::from_millis(150),
            "Expected ~100ms with concurrency=2, got {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_retry_condition_transient_only() {
        let executor = TaskExecutor::new(4);
        // Task that fails with transient errors, retries should work
        let task = TransientFailTask::new(
            "transient_task",
            2,
            RetryPolicy::fixed(3, Duration::from_millis(1))
                .with_condition(RetryCondition::TransientOnly),
        );
        let mut ctx = create_test_context();

        let result = executor.execute(&task, &mut ctx).await;

        assert!(result.success);
        assert_eq!(result.attempts, 3);
    }

    #[tokio::test]
    async fn test_retry_condition_transient_only_non_transient_fails_immediately() {
        let executor = TaskExecutor::new(4);
        // Task that fails with non-transient errors, retry should not happen
        let task = FailingTask {
            name: "non_transient_task".to_string(),
            retry_policy: RetryPolicy::fixed(3, Duration::from_millis(1))
                .with_condition(RetryCondition::TransientOnly),
        };
        let mut ctx = create_test_context();

        let result = executor.execute(&task, &mut ctx).await;

        assert!(!result.success);
        assert_eq!(result.attempts, 1); // No retries for non-transient errors
    }

    #[tokio::test]
    async fn test_retry_condition_never() {
        let executor = TaskExecutor::new(4);
        let task = CountingTask::new(
            "no_retry_task",
            RetryPolicy::fixed(5, Duration::from_millis(1))
                .with_condition(RetryCondition::Never),
        );
        let mut ctx = create_test_context();

        let result = executor.execute(&task, &mut ctx).await;

        assert!(!result.success);
        assert_eq!(result.attempts, 1); // No retries when condition is Never
        assert_eq!(task.count(), 1);
    }

    #[tokio::test]
    async fn test_task_result_includes_duration() {
        let executor = TaskExecutor::new(4);
        let task = SlowTask {
            name: "timed_task".to_string(),
            duration: Duration::from_millis(50),
        };
        let mut ctx = create_test_context();

        let result = executor.execute(&task, &mut ctx).await;

        assert!(result.success);
        assert!(result.duration >= Duration::from_millis(50));
    }

    #[tokio::test]
    async fn test_available_permits() {
        let executor = TaskExecutor::new(4);

        assert_eq!(executor.available_permits(), 4);
        assert_eq!(executor.max_concurrency(), 4);
    }

    #[tokio::test]
    async fn test_default_executor() {
        let executor = TaskExecutor::default();

        assert_eq!(executor.max_concurrency(), 4);
    }
}
