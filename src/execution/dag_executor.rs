//! DAG execution engine.
//!
//! The `DagExecutor` orchestrates the execution of a DAG, running tasks
//! in dependency order with parallel execution of independent tasks.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{Instrument, debug, info_span};

use crate::core::context::TaskContext;
use crate::core::dag::{Dag, TaskCondition};
use crate::core::types::TaskId;
use crate::events::{Event, EventBus};

use super::executor::{TaskExecutor, TaskResult};

/// Status of a task within a DAG execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskStatus {
    /// Task is waiting for dependencies.
    Pending,
    /// Task is currently executing.
    Running,
    /// Task completed successfully.
    Completed,
    /// Task failed after all retries.
    Failed,
    /// Task was skipped (condition not met or upstream failed).
    Skipped,
}

/// Result of executing a DAG.
#[derive(Debug, Clone)]
pub struct DagResult {
    /// Whether the entire DAG succeeded.
    pub success: bool,
    /// Total duration of DAG execution.
    pub duration: std::time::Duration,
    /// Results for each task.
    pub task_results: HashMap<TaskId, TaskResult>,
    /// Final status of each task.
    pub task_statuses: HashMap<TaskId, TaskStatus>,
    /// Tasks that failed.
    pub failed_tasks: Vec<TaskId>,
    /// Tasks that were skipped.
    pub skipped_tasks: Vec<TaskId>,
}

impl DagResult {
    /// Get the number of completed tasks.
    pub fn completed_count(&self) -> usize {
        self.task_statuses
            .values()
            .filter(|s| **s == TaskStatus::Completed)
            .count()
    }

    /// Get the number of failed tasks.
    pub fn failed_count(&self) -> usize {
        self.failed_tasks.len()
    }

    /// Get the number of skipped tasks.
    pub fn skipped_count(&self) -> usize {
        self.skipped_tasks.len()
    }

    /// Get the result for a specific task.
    pub fn get_task_result(&self, task_id: &TaskId) -> Option<&TaskResult> {
        self.task_results.get(task_id)
    }
}

/// Executor for running DAGs with dependency management.
pub struct DagExecutor {
    /// Underlying task executor.
    task_executor: Arc<TaskExecutor>,
}

impl DagExecutor {
    /// Create a new DAG executor with the given task executor.
    pub fn new(task_executor: TaskExecutor) -> Self {
        Self {
            task_executor: Arc::new(task_executor),
        }
    }

    /// Create a new DAG executor with default settings.
    pub fn with_concurrency(max_concurrency: usize) -> Self {
        Self::new(TaskExecutor::new(max_concurrency))
    }

    /// Execute a DAG, respecting dependencies and conditions.
    ///
    /// Tasks are executed in topological order, with independent tasks
    /// running in parallel up to the executor's concurrency limit.
    ///
    /// If an EventBus is provided, task-level events (TaskStarted, TaskCompleted,
    /// TaskFailed) will be emitted.
    pub async fn execute(&self, dag: &Dag, ctx: &mut TaskContext) -> DagResult {
        self.execute_with_events(dag, ctx, None).await
    }

    /// Execute a DAG with optional event emission.
    pub async fn execute_with_events(
        &self,
        dag: &Dag,
        ctx: &mut TaskContext,
        event_bus: Option<Arc<EventBus>>,
    ) -> DagResult {
        let dag_id = dag.id().clone();
        let task_count = dag.task_ids().len();

        let span = info_span!(
            "dag_execution",
            dag = %dag_id,
            task_count = task_count,
        );
        let _enter = span.enter();

        debug!(dag = %dag_id, task_count = task_count, "starting DAG execution");

        let start_time = Instant::now();

        // Track task statuses
        let statuses: Arc<RwLock<HashMap<TaskId, TaskStatus>>> = Arc::new(RwLock::new(
            dag.task_ids()
                .into_iter()
                .map(|id| (id, TaskStatus::Pending))
                .collect(),
        ));

        // Track task results
        let results: Arc<RwLock<HashMap<TaskId, TaskResult>>> =
            Arc::new(RwLock::new(HashMap::new()));

        // Shared context for all tasks
        let shared_store = ctx.inputs.clone();

        // Execute tasks in waves until all are done
        loop {
            // Find tasks that are ready to run
            let ready_tasks = self.get_ready_tasks(dag, &statuses).await;

            if ready_tasks.is_empty() {
                // Check if we're done or deadlocked
                let statuses_read = statuses.read().await;
                let all_done = statuses_read.values().all(|s| {
                    matches!(
                        s,
                        TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Skipped
                    )
                });

                if all_done {
                    debug!(dag = %dag_id, "all tasks completed");
                    break;
                }

                // Check for tasks that should be skipped due to failed dependencies
                drop(statuses_read);
                let skipped = self.skip_blocked_tasks(dag, &statuses).await;
                if skipped == 0 {
                    // No progress possible, break to avoid infinite loop
                    debug!(dag = %dag_id, "no progress possible, breaking execution loop");
                    break;
                }
                continue;
            }

            debug!(
                dag = %dag_id,
                ready_count = ready_tasks.len(),
                "dispatching ready tasks"
            );

            // Execute ready tasks in parallel
            let mut handles = vec![];

            for task_id in ready_tasks {
                let dag_clone = dag.clone();
                let dag_id = dag.id().clone();
                let statuses_clone = Arc::clone(&statuses);
                let results_clone = Arc::clone(&results);
                let executor = Arc::clone(&self.task_executor);
                let store = shared_store.clone();
                let config = ctx.config.clone();
                let event_bus = event_bus.clone();

                let task_span = info_span!(
                    "dag_task_dispatch",
                    dag = %dag_id,
                    task = %task_id,
                );

                handles.push(tokio::spawn(
                    async move {
                        // Mark as running
                        {
                            let mut statuses_write = statuses_clone.write().await;
                            statuses_write.insert(task_id.clone(), TaskStatus::Running);
                        }

                        // Emit TaskStarted event
                        if let Some(ref bus) = event_bus {
                            bus.emit(Event::task_started(task_id.clone(), dag_id.clone()))
                                .await;
                        }

                        // Get the task
                        let node = dag_clone.get_task(&task_id).unwrap();
                        let task = &node.task;

                        // Create task context
                        let task_store = Arc::new(std::sync::RwLock::new(
                            store.clone_store().unwrap_or_default(),
                        ));
                        let mut task_ctx =
                            TaskContext::new(task_store.clone(), task_id.clone(), config);

                        // Execute (uses semaphore for concurrency control)
                        let task_start = Instant::now();
                        let result = match executor.execute(task.as_ref(), &mut task_ctx).await {
                            Ok(result) => result,
                            Err(e) => TaskResult::failure(
                                task_id.clone(),
                                0,
                                task_start.elapsed(),
                                e.to_string(),
                            ),
                        };
                        let task_duration = task_start.elapsed();

                        // Merge outputs back to shared store
                        if let Ok(outputs) = task_store.read() {
                            for (key, value) in outputs.iter() {
                                let _ = store.set_raw(key, value.clone());
                            }
                        }

                        // Update status based on result
                        let status = if result.success {
                            TaskStatus::Completed
                        } else {
                            TaskStatus::Failed
                        };
                        {
                            let mut statuses_write = statuses_clone.write().await;
                            statuses_write.insert(task_id.clone(), status.clone());
                        }

                        // Read stdout/stderr/exit_code from context
                        let stdout: Option<String> = task_store
                            .read()
                            .ok()
                            .and_then(|s| s.get(&format!("{}.stdout", task_id.as_str())).cloned())
                            .and_then(|v| serde_json::from_value(v).ok());
                        let stderr: Option<String> = task_store
                            .read()
                            .ok()
                            .and_then(|s| s.get(&format!("{}.stderr", task_id.as_str())).cloned())
                            .and_then(|v| serde_json::from_value(v).ok());
                        let exit_code: Option<i32> = task_store
                            .read()
                            .ok()
                            .and_then(|s| {
                                s.get(&format!("{}.exit_code", task_id.as_str())).cloned()
                            })
                            .and_then(|v| serde_json::from_value(v).ok());

                        // Emit TaskCompleted or TaskFailed event
                        if let Some(ref bus) = event_bus {
                            if result.success {
                                bus.emit(Event::task_completed_with_output(
                                    task_id.clone(),
                                    dag_id,
                                    task_duration,
                                    stdout,
                                    stderr,
                                    exit_code,
                                ))
                                .await;
                            } else {
                                bus.emit(Event::task_failed_with_output(
                                    task_id.clone(),
                                    dag_id,
                                    result
                                        .error
                                        .clone()
                                        .unwrap_or_else(|| "unknown error".to_string()),
                                    stdout,
                                    stderr,
                                    exit_code,
                                ))
                                .await;
                            }
                        }

                        // Store result
                        {
                            let mut results_write = results_clone.write().await;
                            results_write.insert(task_id.clone(), result);
                        }
                    }
                    .instrument(task_span),
                ));
            }

            // Wait for all tasks in this wave to complete
            for handle in handles {
                let _ = handle.await;
            }
        }

        // Collect final results
        let statuses_final = statuses.read().await;
        let results_final = results.read().await;

        let failed_tasks: Vec<TaskId> = statuses_final
            .iter()
            .filter(|(_, s)| **s == TaskStatus::Failed)
            .map(|(id, _)| id.clone())
            .collect();

        let skipped_tasks: Vec<TaskId> = statuses_final
            .iter()
            .filter(|(_, s)| **s == TaskStatus::Skipped)
            .map(|(id, _)| id.clone())
            .collect();

        let success = failed_tasks.is_empty() && skipped_tasks.is_empty();
        let duration = start_time.elapsed();

        debug!(
            dag = %dag_id,
            success = success,
            duration_ms = %duration.as_millis(),
            completed = results_final.len(),
            failed = failed_tasks.len(),
            skipped = skipped_tasks.len(),
            "DAG execution completed"
        );

        DagResult {
            success,
            duration,
            task_results: results_final.clone(),
            task_statuses: statuses_final.clone(),
            failed_tasks,
            skipped_tasks,
        }
    }

    /// Find tasks that are ready to execute.
    async fn get_ready_tasks(
        &self,
        dag: &Dag,
        statuses: &Arc<RwLock<HashMap<TaskId, TaskStatus>>>,
    ) -> Vec<TaskId> {
        let statuses_read = statuses.read().await;
        let mut ready = vec![];

        for task_id in dag.task_ids() {
            // Skip if not pending
            if statuses_read.get(&task_id) != Some(&TaskStatus::Pending) {
                continue;
            }

            // Check if all dependencies are satisfied
            let deps = dag.get_dependencies(&task_id).unwrap_or(&[]);
            let deps_satisfied = deps.iter().all(|dep_id| {
                matches!(
                    statuses_read.get(dep_id),
                    Some(TaskStatus::Completed)
                        | Some(TaskStatus::Failed)
                        | Some(TaskStatus::Skipped)
                )
            });

            if !deps_satisfied {
                continue;
            }

            // Check task condition
            let node = dag.get_task(&task_id).unwrap();
            let should_run = self.evaluate_condition(&node.condition, deps, &statuses_read);

            if should_run {
                ready.push(task_id.clone());
            }
        }

        ready
    }

    /// Evaluate whether a task should run based on its condition and upstream results.
    fn evaluate_condition(
        &self,
        condition: &TaskCondition,
        dependencies: &[TaskId],
        statuses: &HashMap<TaskId, TaskStatus>,
    ) -> bool {
        match condition {
            TaskCondition::Always => true,
            TaskCondition::AllSuccess => dependencies
                .iter()
                .all(|dep| statuses.get(dep) == Some(&TaskStatus::Completed)),
            TaskCondition::OnFailure => dependencies
                .iter()
                .any(|dep| statuses.get(dep) == Some(&TaskStatus::Failed)),
            TaskCondition::AllDone => dependencies.iter().all(|dep| {
                matches!(
                    statuses.get(dep),
                    Some(TaskStatus::Completed)
                        | Some(TaskStatus::Failed)
                        | Some(TaskStatus::Skipped)
                )
            }),
        }
    }

    /// Skip tasks that are blocked due to failed dependencies.
    async fn skip_blocked_tasks(
        &self,
        dag: &Dag,
        statuses: &Arc<RwLock<HashMap<TaskId, TaskStatus>>>,
    ) -> usize {
        let mut statuses_write = statuses.write().await;
        let mut skipped_count = 0;

        for task_id in dag.task_ids() {
            if statuses_write.get(&task_id) != Some(&TaskStatus::Pending) {
                continue;
            }

            let deps = dag.get_dependencies(&task_id).unwrap_or(&[]);
            let node = dag.get_task(&task_id).unwrap();

            // Check if all deps are done
            let deps_done = deps.iter().all(|dep_id| {
                matches!(
                    statuses_write.get(dep_id),
                    Some(TaskStatus::Completed)
                        | Some(TaskStatus::Failed)
                        | Some(TaskStatus::Skipped)
                )
            });

            if !deps_done {
                continue;
            }

            // Check if condition allows running
            let should_run = self.evaluate_condition(&node.condition, deps, &statuses_write);

            if !should_run {
                statuses_write.insert(task_id.clone(), TaskStatus::Skipped);
                skipped_count += 1;
            }
        }

        skipped_count
    }
}

impl Default for DagExecutor {
    fn default() -> Self {
        Self::with_concurrency(4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::dag::DagBuilder;
    use crate::core::retry::RetryPolicy;
    use crate::core::task::{Task, TaskError};
    use crate::core::types::TaskId;
    use async_trait::async_trait;
    use serde_json::Value;
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    fn create_shared_context() -> TaskContext {
        let store = Arc::new(std::sync::RwLock::new(HashMap::<String, Value>::new()));
        let config = Arc::new(HashMap::new());
        TaskContext::new(store, TaskId::new("dag"), config)
    }

    // Simple task that succeeds and writes to context
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

    // Task that always fails
    struct FailingTask {
        name: String,
    }

    impl FailingTask {
        fn new(name: &str) -> Arc<Self> {
            Arc::new(Self {
                name: name.to_string(),
            })
        }
    }

    #[async_trait]
    impl Task for FailingTask {
        fn name(&self) -> &str {
            &self.name
        }

        async fn execute(&self, _ctx: &mut TaskContext) -> Result<(), TaskError> {
            Err(TaskError::ExecutionFailed(
                "intentional failure".to_string(),
            ))
        }

        fn retry_policy(&self) -> RetryPolicy {
            RetryPolicy::none() // Don't retry
        }
    }

    // Task that reads from upstream and writes transformed data
    struct TransformTask {
        name: String,
        upstream_key: String,
    }

    impl TransformTask {
        fn new(name: &str, upstream_key: &str) -> Arc<Self> {
            Arc::new(Self {
                name: name.to_string(),
                upstream_key: upstream_key.to_string(),
            })
        }
    }

    #[async_trait]
    impl Task for TransformTask {
        fn name(&self) -> &str {
            &self.name
        }

        async fn execute(&self, ctx: &mut TaskContext) -> Result<(), TaskError> {
            let input: i32 = ctx.inputs.get(&self.upstream_key)?;
            ctx.outputs.set("value", input * 2)?;
            Ok(())
        }
    }

    // Task that writes a specific value
    struct WriteTask {
        name: String,
        value: i32,
    }

    impl WriteTask {
        fn new(name: &str, value: i32) -> Arc<Self> {
            Arc::new(Self {
                name: name.to_string(),
                value,
            })
        }
    }

    #[async_trait]
    impl Task for WriteTask {
        fn name(&self) -> &str {
            &self.name
        }

        async fn execute(&self, ctx: &mut TaskContext) -> Result<(), TaskError> {
            ctx.outputs.set("value", self.value)?;
            Ok(())
        }
    }

    // Slow task for testing parallelism
    struct SlowTask {
        name: String,
        duration: Duration,
        call_order: Arc<AtomicU32>,
        my_order: Arc<AtomicU32>,
    }

    impl SlowTask {
        fn new(name: &str, duration: Duration, call_order: Arc<AtomicU32>) -> Arc<Self> {
            Arc::new(Self {
                name: name.to_string(),
                duration,
                call_order,
                my_order: Arc::new(AtomicU32::new(0)),
            })
        }

        fn get_order(&self) -> u32 {
            self.my_order.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl Task for SlowTask {
        fn name(&self) -> &str {
            &self.name
        }

        async fn execute(&self, _ctx: &mut TaskContext) -> Result<(), TaskError> {
            let order = self.call_order.fetch_add(1, Ordering::SeqCst);
            self.my_order.store(order, Ordering::SeqCst);
            tokio::time::sleep(self.duration).await;
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_execute_single_task_dag() {
        let executor = DagExecutor::with_concurrency(4);
        let task = SimpleTask::new("only_task");

        let dag = DagBuilder::new("single", "Single Task DAG")
            .add_task(task)
            .build()
            .unwrap();

        let mut ctx = create_shared_context();
        let result = executor.execute(&dag, &mut ctx).await;

        assert!(result.success);
        assert_eq!(result.completed_count(), 1);
        assert_eq!(result.failed_count(), 0);
    }

    #[tokio::test]
    async fn test_execute_linear_dag() {
        // A -> B -> C
        let executor = DagExecutor::with_concurrency(4);

        let task_a = SimpleTask::new("a");
        let task_b = SimpleTask::new("b");
        let task_c = SimpleTask::new("c");

        let dag = DagBuilder::new("linear", "Linear DAG")
            .add_task(task_a)
            .add_task_with_deps(task_b, &["a"])
            .add_task_with_deps(task_c, &["b"])
            .build()
            .unwrap();

        let mut ctx = create_shared_context();
        let result = executor.execute(&dag, &mut ctx).await;

        assert!(result.success);
        assert_eq!(result.completed_count(), 3);
    }

    #[tokio::test]
    async fn test_execute_diamond_dag() {
        //     A
        //    / \
        //   B   C
        //    \ /
        //     D
        let executor = DagExecutor::with_concurrency(4);

        let task_a = SimpleTask::new("a");
        let task_b = SimpleTask::new("b");
        let task_c = SimpleTask::new("c");
        let task_d = SimpleTask::new("d");

        let dag = DagBuilder::new("diamond", "Diamond DAG")
            .add_task(task_a)
            .add_task_with_deps(task_b, &["a"])
            .add_task_with_deps(task_c, &["a"])
            .add_task_with_deps(task_d, &["b", "c"])
            .build()
            .unwrap();

        let mut ctx = create_shared_context();
        let result = executor.execute(&dag, &mut ctx).await;

        assert!(result.success);
        assert_eq!(result.completed_count(), 4);
    }

    #[tokio::test]
    async fn test_parallel_execution_of_independent_tasks() {
        let executor = DagExecutor::with_concurrency(4);
        let call_order = Arc::new(AtomicU32::new(0));

        // A runs first, then B and C should run in parallel
        let task_a = SlowTask::new("a", Duration::from_millis(10), Arc::clone(&call_order));
        let task_b = SlowTask::new("b", Duration::from_millis(50), Arc::clone(&call_order));
        let task_c = SlowTask::new("c", Duration::from_millis(50), Arc::clone(&call_order));

        let dag = DagBuilder::new("parallel", "Parallel DAG")
            .add_task(Arc::clone(&task_a) as Arc<dyn Task>)
            .add_task_with_deps(Arc::clone(&task_b) as Arc<dyn Task>, &["a"])
            .add_task_with_deps(Arc::clone(&task_c) as Arc<dyn Task>, &["a"])
            .build()
            .unwrap();

        let mut ctx = create_shared_context();
        let start = Instant::now();
        let result = executor.execute(&dag, &mut ctx).await;
        let elapsed = start.elapsed();

        assert!(result.success);

        // A should be first (order 0)
        assert_eq!(task_a.get_order(), 0);

        // B and C should start around the same time (orders 1 and 2)
        let b_order = task_b.get_order();
        let c_order = task_c.get_order();
        assert!(b_order >= 1 && b_order <= 2);
        assert!(c_order >= 1 && c_order <= 2);

        // Total time should be ~60ms (10 for A + 50 for B||C), not ~110ms
        assert!(
            elapsed < Duration::from_millis(100),
            "Expected parallel execution, got {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_task_failure_stops_downstream_with_all_success() {
        // A (fails) -> B (should be skipped)
        let executor = DagExecutor::with_concurrency(4);

        let task_a = FailingTask::new("a");
        let task_b = SimpleTask::new("b");

        let dag = DagBuilder::new("failure", "Failure DAG")
            .add_task(task_a)
            .add_task_with_deps_and_condition(task_b, &["a"], TaskCondition::AllSuccess)
            .build()
            .unwrap();

        let mut ctx = create_shared_context();
        let result = executor.execute(&dag, &mut ctx).await;

        assert!(!result.success);
        assert_eq!(result.failed_count(), 1);
        assert_eq!(result.skipped_count(), 1);
        assert_eq!(
            result.task_statuses.get(&TaskId::new("b")),
            Some(&TaskStatus::Skipped)
        );
    }

    #[tokio::test]
    async fn test_on_failure_condition_runs_on_upstream_failure() {
        // A (fails) -> B (OnFailure, should run)
        let executor = DagExecutor::with_concurrency(4);

        let task_a = FailingTask::new("a");
        let task_b = SimpleTask::new("b");

        let dag = DagBuilder::new("on_failure", "OnFailure DAG")
            .add_task(task_a)
            .add_task_with_deps_and_condition(task_b, &["a"], TaskCondition::OnFailure)
            .build()
            .unwrap();

        let mut ctx = create_shared_context();
        let result = executor.execute(&dag, &mut ctx).await;

        // DAG still fails because A failed, but B ran
        assert!(!result.success);
        assert_eq!(result.failed_count(), 1);
        assert_eq!(
            result.task_statuses.get(&TaskId::new("b")),
            Some(&TaskStatus::Completed)
        );
    }

    #[tokio::test]
    async fn test_on_failure_condition_skipped_when_upstream_succeeds() {
        // A (succeeds) -> B (OnFailure, should be skipped)
        let executor = DagExecutor::with_concurrency(4);

        let task_a = SimpleTask::new("a");
        let task_b = SimpleTask::new("b");

        let dag = DagBuilder::new("on_failure_skip", "OnFailure Skip DAG")
            .add_task(task_a)
            .add_task_with_deps_and_condition(task_b, &["a"], TaskCondition::OnFailure)
            .build()
            .unwrap();

        let mut ctx = create_shared_context();
        let result = executor.execute(&dag, &mut ctx).await;

        // B was skipped, but that's expected for OnFailure when upstream succeeds
        // We consider this a success since no task actually failed
        assert_eq!(
            result.task_statuses.get(&TaskId::new("a")),
            Some(&TaskStatus::Completed)
        );
        assert_eq!(
            result.task_statuses.get(&TaskId::new("b")),
            Some(&TaskStatus::Skipped)
        );
    }

    #[tokio::test]
    async fn test_all_done_condition_runs_regardless_of_status() {
        // A (fails) -> B (AllDone, should run)
        let executor = DagExecutor::with_concurrency(4);

        let task_a = FailingTask::new("a");
        let task_b = SimpleTask::new("b");

        let dag = DagBuilder::new("all_done", "AllDone DAG")
            .add_task(task_a)
            .add_task_with_deps_and_condition(task_b, &["a"], TaskCondition::AllDone)
            .build()
            .unwrap();

        let mut ctx = create_shared_context();
        let result = executor.execute(&dag, &mut ctx).await;

        assert_eq!(
            result.task_statuses.get(&TaskId::new("a")),
            Some(&TaskStatus::Failed)
        );
        assert_eq!(
            result.task_statuses.get(&TaskId::new("b")),
            Some(&TaskStatus::Completed)
        );
    }

    #[tokio::test]
    async fn test_context_flows_from_upstream_to_downstream() {
        // A writes value -> B reads and transforms
        let executor = DagExecutor::with_concurrency(4);

        let task_a = WriteTask::new("a", 21);
        let task_b = TransformTask::new("b", "a.value");

        let dag = DagBuilder::new("context_flow", "Context Flow DAG")
            .add_task(task_a)
            .add_task_with_deps(task_b, &["a"])
            .build()
            .unwrap();

        let mut ctx = create_shared_context();
        let result = executor.execute(&dag, &mut ctx).await;

        assert!(result.success);

        // Check that B's output is in context (21 * 2 = 42)
        let value: i32 = ctx.inputs.get("b.value").unwrap();
        assert_eq!(value, 42);
    }

    #[tokio::test]
    async fn test_dag_execution_respects_concurrency_limit() {
        let executor = DagExecutor::with_concurrency(1); // Only 1 at a time
        let call_order = Arc::new(AtomicU32::new(0));

        // Three independent tasks
        let task_a = SlowTask::new("a", Duration::from_millis(20), Arc::clone(&call_order));
        let task_b = SlowTask::new("b", Duration::from_millis(20), Arc::clone(&call_order));
        let task_c = SlowTask::new("c", Duration::from_millis(20), Arc::clone(&call_order));

        let dag = DagBuilder::new("sequential", "Sequential DAG")
            .add_task(Arc::clone(&task_a) as Arc<dyn Task>)
            .add_task(Arc::clone(&task_b) as Arc<dyn Task>)
            .add_task(Arc::clone(&task_c) as Arc<dyn Task>)
            .build()
            .unwrap();

        let mut ctx = create_shared_context();
        let start = Instant::now();
        let result = executor.execute(&dag, &mut ctx).await;
        let elapsed = start.elapsed();

        assert!(result.success);

        // With concurrency=1, tasks run sequentially
        // Each task should have a different order
        let orders: HashSet<_> = [task_a.get_order(), task_b.get_order(), task_c.get_order()]
            .into_iter()
            .collect();
        assert_eq!(orders.len(), 3, "All tasks should have unique orders");

        // Total time should be ~60ms (3 * 20ms sequentially)
        assert!(
            elapsed >= Duration::from_millis(50),
            "Expected sequential execution, got {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_collect_all_task_results() {
        let executor = DagExecutor::with_concurrency(4);

        let task_a = SimpleTask::new("a");
        let task_b = SimpleTask::new("b");

        let dag = DagBuilder::new("results", "Results DAG")
            .add_task(task_a)
            .add_task_with_deps(task_b, &["a"])
            .build()
            .unwrap();

        let mut ctx = create_shared_context();
        let result = executor.execute(&dag, &mut ctx).await;

        assert!(result.success);

        // Both tasks should have results
        let result_a = result.get_task_result(&TaskId::new("a")).unwrap();
        let result_b = result.get_task_result(&TaskId::new("b")).unwrap();

        assert!(result_a.success);
        assert!(result_b.success);
        assert_eq!(result_a.attempts, 1);
        assert_eq!(result_b.attempts, 1);
    }
}
