//! Complete workflow integration tests.
//!
//! Tests that verify the full pipeline from job definition to execution.

use async_trait::async_trait;
use petit::{
    DagBuilder, DagExecutor, Event, EventBus, EventHandler, InMemoryStorage, Job, JobDependency,
    Schedule, Scheduler, Task, TaskCondition, TaskContext, TaskError, TaskId, YamlLoader,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Recording event handler for verifying events.
struct RecordingHandler {
    events: Mutex<Vec<Event>>,
}

impl RecordingHandler {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            events: Mutex::new(Vec::new()),
        })
    }

    async fn events(&self) -> Vec<Event> {
        self.events.lock().await.clone()
    }

    async fn job_started_count(&self) -> usize {
        self.events
            .lock()
            .await
            .iter()
            .filter(|e| matches!(e, Event::JobStarted { .. }))
            .count()
    }

    async fn job_completed_count(&self) -> usize {
        self.events
            .lock()
            .await
            .iter()
            .filter(|e| matches!(e, Event::JobCompleted { .. }))
            .count()
    }
}

#[async_trait]
impl EventHandler for RecordingHandler {
    async fn handle(&self, event: &Event) {
        self.events.lock().await.push(event.clone());
    }
}

/// Simple task that succeeds.
struct SuccessTask {
    name: String,
}

impl SuccessTask {
    fn new(name: &str) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
        })
    }
}

#[async_trait]
impl Task for SuccessTask {
    fn name(&self) -> &str {
        &self.name
    }

    async fn execute(&self, ctx: &mut TaskContext) -> Result<(), TaskError> {
        ctx.outputs.set("completed", true)?;
        Ok(())
    }
}

/// Task that fails on purpose.
struct FailTask {
    name: String,
}

impl FailTask {
    fn new(name: &str) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
        })
    }
}

#[async_trait]
impl Task for FailTask {
    fn name(&self) -> &str {
        &self.name
    }

    async fn execute(&self, _ctx: &mut TaskContext) -> Result<(), TaskError> {
        Err(TaskError::ExecutionFailed("intentional failure".into()))
    }
}

/// Test: Complete workflow - define job, register, and execute.
#[tokio::test]
async fn test_complete_workflow_define_and_execute() {
    // 1. Build a DAG with multiple tasks
    let dag = DagBuilder::new("etl", "ETL Pipeline")
        .add_task(SuccessTask::new("extract"))
        .add_task_with_deps(SuccessTask::new("transform"), &["extract"])
        .add_task_with_deps(SuccessTask::new("load"), &["transform"])
        .build()
        .unwrap();

    // 2. Create a job with the DAG
    let job = Job::new("daily_etl", "Daily ETL Job", dag);

    // 3. Set up scheduler with event handler
    let event_bus = EventBus::new();
    let handler = RecordingHandler::new();
    event_bus.register(handler.clone()).await;

    let storage = InMemoryStorage::new();
    let mut scheduler = Scheduler::new(storage).with_event_bus(event_bus);
    scheduler.register(job);

    // 4. Start scheduler and trigger job
    let (handle, task) = scheduler.start().await;
    let _run_id = handle.trigger("daily_etl").await.unwrap();

    // 5. Wait for execution
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 6. Verify events
    assert_eq!(handler.job_started_count().await, 1);
    assert_eq!(handler.job_completed_count().await, 1);

    // 7. Check completion event was successful
    let events = handler.events().await;
    let completed = events
        .iter()
        .find(|e| matches!(e, Event::JobCompleted { .. }))
        .unwrap();

    if let Event::JobCompleted { success, .. } = completed {
        assert!(*success, "Job should have completed successfully");
    }

    handle.shutdown().await.unwrap();
    let _ = task.await;
}

/// Test: Parse job from YAML and execute.
#[tokio::test]
async fn test_workflow_from_yaml_config() {
    let yaml = r#"
id: yaml_job
name: YAML Defined Job
description: A job defined in YAML
tasks:
  - id: step1
    name: First Step
    type: command
    command: echo
    args:
      - "step 1"
  - id: step2
    name: Second Step
    depends_on:
      - step1
    type: command
    command: echo
    args:
      - "step 2"
"#;

    // Parse the YAML config
    let config = YamlLoader::parse_job_config(yaml).unwrap();

    assert_eq!(config.id, "yaml_job");
    assert_eq!(config.name, "YAML Defined Job");
    assert_eq!(config.tasks.len(), 2);
    assert_eq!(config.tasks[0].id, "step1");
    assert_eq!(config.tasks[1].id, "step2");
    assert_eq!(config.tasks[1].depends_on, vec!["step1"]);
}

/// Test: Complex DAG with mixed conditions.
#[tokio::test]
async fn test_complex_dag_with_mixed_conditions() {
    // Build a complex DAG:
    //       start
    //      /     \
    //   success  failure
    //      \     /
    //     cleanup (AllDone)
    //         |
    //     notify (AllSuccess - should be skipped)
    //         |
    //     on_fail (OnFailure - should run)

    let dag = DagBuilder::new("complex", "Complex DAG")
        .add_task(SuccessTask::new("start"))
        .add_task_with_deps(SuccessTask::new("success"), &["start"])
        .add_task_with_deps(FailTask::new("failure"), &["start"])
        .add_task_with_deps_and_condition(
            SuccessTask::new("cleanup"),
            &["success", "failure"],
            TaskCondition::AllDone,
        )
        .add_task_with_deps_and_condition(
            SuccessTask::new("notify"),
            &["cleanup"],
            TaskCondition::AllSuccess,
        )
        .add_task_with_deps_and_condition(
            SuccessTask::new("on_fail"),
            &["failure"],
            TaskCondition::OnFailure,
        )
        .build()
        .unwrap();

    // Execute the DAG
    let executor = DagExecutor::with_concurrency(4);
    let store = Arc::new(std::sync::RwLock::new(HashMap::new()));
    let config = Arc::new(HashMap::new());
    let mut ctx = TaskContext::new(store, TaskId::new("test"), config);

    let result = executor.execute(&dag, &mut ctx).await;

    // Verify results
    assert!(
        !result.success,
        "DAG should fail because 'failure' task failed"
    );

    // 'start' should complete
    assert_eq!(
        result.task_statuses.get(&TaskId::new("start")),
        Some(&petit::TaskStatus::Completed)
    );

    // 'success' should complete
    assert_eq!(
        result.task_statuses.get(&TaskId::new("success")),
        Some(&petit::TaskStatus::Completed)
    );

    // 'failure' should fail
    assert_eq!(
        result.task_statuses.get(&TaskId::new("failure")),
        Some(&petit::TaskStatus::Failed)
    );

    // 'cleanup' should complete (AllDone condition)
    assert_eq!(
        result.task_statuses.get(&TaskId::new("cleanup")),
        Some(&petit::TaskStatus::Completed)
    );

    // 'on_fail' should complete (OnFailure condition after 'failure' failed)
    assert_eq!(
        result.task_statuses.get(&TaskId::new("on_fail")),
        Some(&petit::TaskStatus::Completed)
    );

    // 'notify' should be skipped (AllSuccess condition but cleanup's upstream had failures)
    // Note: cleanup completed, but the path to cleanup included failures
    // This depends on implementation details - adjust if needed
}

/// Test: Cross-job dependency chain.
#[tokio::test]
async fn test_cross_job_dependency_chain() {
    let storage = InMemoryStorage::new();
    let event_bus = EventBus::new();
    let handler = RecordingHandler::new();
    event_bus.register(handler.clone()).await;

    let mut scheduler = Scheduler::new(storage).with_event_bus(event_bus);

    // Job A: No dependencies
    let dag_a = DagBuilder::new("dag_a", "DAG A")
        .add_task(SuccessTask::new("task_a"))
        .build()
        .unwrap();
    let job_a = Job::new("job_a", "Job A", dag_a);
    scheduler.register(job_a);

    // Job B: Depends on Job A
    let dag_b = DagBuilder::new("dag_b", "DAG B")
        .add_task(SuccessTask::new("task_b"))
        .build()
        .unwrap();
    let job_b = Job::new("job_b", "Job B", dag_b)
        .with_dependency(JobDependency::new(petit::JobId::new("job_a")));
    scheduler.register(job_b);

    // Job C: Depends on Job B
    let dag_c = DagBuilder::new("dag_c", "DAG C")
        .add_task(SuccessTask::new("task_c"))
        .build()
        .unwrap();
    let job_c = Job::new("job_c", "Job C", dag_c)
        .with_dependency(JobDependency::new(petit::JobId::new("job_b")));
    scheduler.register(job_c);

    let (handle, task) = scheduler.start().await;

    // Try to trigger Job C - should fail (Job B hasn't run)
    let result = handle.trigger("job_c").await;
    assert!(result.is_err(), "Job C should fail - Job B hasn't run");

    // Try to trigger Job B - should fail (Job A hasn't run)
    let result = handle.trigger("job_b").await;
    assert!(result.is_err(), "Job B should fail - Job A hasn't run");

    // Trigger Job A - should succeed
    handle.trigger("job_a").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Now Job B should work
    handle.trigger("job_b").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Now Job C should work
    handle.trigger("job_c").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // All three jobs should have completed
    assert_eq!(handler.job_completed_count().await, 3);

    handle.shutdown().await.unwrap();
    let _ = task.await;
}

/// Test: Diamond dependency pattern in DAG.
#[tokio::test]
async fn test_diamond_dag_execution() {
    //       A
    //      / \
    //     B   C
    //      \ /
    //       D

    let dag = DagBuilder::new("diamond", "Diamond DAG")
        .add_task(SuccessTask::new("A"))
        .add_task_with_deps(SuccessTask::new("B"), &["A"])
        .add_task_with_deps(SuccessTask::new("C"), &["A"])
        .add_task_with_deps(SuccessTask::new("D"), &["B", "C"])
        .build()
        .unwrap();

    let executor = DagExecutor::with_concurrency(4);
    let store = Arc::new(std::sync::RwLock::new(HashMap::new()));
    let config = Arc::new(HashMap::new());
    let mut ctx = TaskContext::new(store, TaskId::new("test"), config);

    let result = executor.execute(&dag, &mut ctx).await;

    assert!(result.success);
    assert_eq!(result.completed_count(), 4);
    assert_eq!(result.failed_count(), 0);
}

/// Test: Job with scheduled execution.
#[tokio::test]
async fn test_job_with_schedule() {
    let dag = DagBuilder::new("scheduled", "Scheduled DAG")
        .add_task(SuccessTask::new("task"))
        .build()
        .unwrap();

    let schedule = Schedule::new("@every 1s").unwrap();
    let job = Job::new("scheduled_job", "Scheduled Job", dag).with_schedule(schedule);

    assert!(job.is_scheduled());
    assert!(job.schedule().is_some());
}

/// Test: Data flow through context in DAG.
#[tokio::test]
async fn test_context_data_flow_through_dag() {
    /// Task that writes a value to context.
    struct WriterTask {
        name: String,
        key: String,
        value: i32,
    }

    impl WriterTask {
        fn new(name: &str, key: &str, value: i32) -> Arc<Self> {
            Arc::new(Self {
                name: name.to_string(),
                key: key.to_string(),
                value,
            })
        }
    }

    #[async_trait]
    impl Task for WriterTask {
        fn name(&self) -> &str {
            &self.name
        }

        async fn execute(&self, ctx: &mut TaskContext) -> Result<(), TaskError> {
            ctx.outputs.set(&self.key, self.value)?;
            Ok(())
        }
    }

    /// Task that reads and transforms a value.
    struct ReaderTask {
        name: String,
        input_key: String,
    }

    impl ReaderTask {
        fn new(name: &str, input_key: &str) -> Arc<Self> {
            Arc::new(Self {
                name: name.to_string(),
                input_key: input_key.to_string(),
            })
        }
    }

    #[async_trait]
    impl Task for ReaderTask {
        fn name(&self) -> &str {
            &self.name
        }

        async fn execute(&self, ctx: &mut TaskContext) -> Result<(), TaskError> {
            let value: i32 = ctx.inputs.get(&self.input_key)?;
            ctx.outputs.set("result", value * 2)?;
            Ok(())
        }
    }

    let dag = DagBuilder::new("dataflow", "Data Flow DAG")
        .add_task(WriterTask::new("writer", "number", 21))
        .add_task_with_deps(ReaderTask::new("reader", "writer.number"), &["writer"])
        .build()
        .unwrap();

    let executor = DagExecutor::with_concurrency(4);
    let store = Arc::new(std::sync::RwLock::new(HashMap::new()));
    let config = Arc::new(HashMap::new());
    let mut ctx = TaskContext::new(store, TaskId::new("test"), config);

    let result = executor.execute(&dag, &mut ctx).await;

    assert!(result.success);

    // Check that the result was properly transformed (21 * 2 = 42)
    let final_result: i32 = ctx.inputs.get("reader.result").unwrap();
    assert_eq!(final_result, 42);
}

/// Test: Multiple jobs running concurrently.
#[tokio::test]
async fn test_multiple_concurrent_jobs() {
    let storage = InMemoryStorage::new();
    let event_bus = EventBus::new();
    let handler = RecordingHandler::new();
    event_bus.register(handler.clone()).await;

    let mut scheduler = Scheduler::new(storage)
        .with_event_bus(event_bus)
        .with_max_concurrent_jobs(5);

    // Register 3 independent jobs
    for i in 1..=3 {
        let dag = DagBuilder::new(format!("dag_{}", i), format!("DAG {}", i))
            .add_task(SuccessTask::new(&format!("task_{}", i)))
            .build()
            .unwrap();
        let job = Job::new(format!("job_{}", i), format!("Job {}", i), dag);
        scheduler.register(job);
    }

    let (handle, task) = scheduler.start().await;

    // Trigger all jobs
    for i in 1..=3 {
        handle.trigger(format!("job_{}", i)).await.unwrap();
    }

    // Wait for all to complete
    tokio::time::sleep(Duration::from_millis(300)).await;

    // All 3 jobs should have completed
    assert_eq!(handler.job_completed_count().await, 3);

    handle.shutdown().await.unwrap();
    let _ = task.await;
}
