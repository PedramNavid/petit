//! Recovery scenario integration tests.
//!
//! Tests that verify the system can recover from interruptions
//! and handle failed runs appropriately.

use petit::{
    DagBuilder, InMemoryStorage, Job, JobId, RunId, RunStatus, Scheduler, Storage, StoredRun,
    Task, TaskContext, TaskError,
};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

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

    async fn execute(&self, _ctx: &mut TaskContext) -> Result<(), TaskError> {
        Ok(())
    }
}

/// Test: Recover interrupted runs on startup.
#[tokio::test]
async fn test_recover_interrupted_runs_on_startup() {
    let storage = Arc::new(InMemoryStorage::new());

    // Simulate interrupted runs by creating runs with Running status
    let run1 = StoredRun::new(RunId::new(), JobId::new("job1"));
    storage.save_run(run1.clone()).await.unwrap();

    let mut run2 = StoredRun::new(RunId::new(), JobId::new("job2"));
    run2.mark_running();
    storage.save_run(run2.clone()).await.unwrap();

    // Create scheduler with shared storage
    let scheduler = Scheduler::with_storage(Arc::clone(&storage));

    // Recover should mark interrupted runs
    let recovered = scheduler.recover().await.unwrap();

    // Both runs should be recovered (one pending, one running)
    assert_eq!(recovered.len(), 2);

    // Check that runs are now marked as interrupted
    let updated_run1 = storage.get_run(&run1.id).await.unwrap();
    let updated_run2 = storage.get_run(&run2.id).await.unwrap();

    assert_eq!(updated_run1.status, RunStatus::Interrupted);
    assert_eq!(updated_run2.status, RunStatus::Interrupted);
}

/// Test: Completed runs are not affected by recovery.
#[tokio::test]
async fn test_completed_runs_not_affected_by_recovery() {
    let storage = Arc::new(InMemoryStorage::new());

    // Create a completed run
    let mut completed_run = StoredRun::new(RunId::new(), JobId::new("job1"));
    completed_run.mark_running();
    completed_run.mark_completed();
    storage.save_run(completed_run.clone()).await.unwrap();

    // Create a failed run
    let mut failed_run = StoredRun::new(RunId::new(), JobId::new("job2"));
    failed_run.mark_running();
    failed_run.mark_failed("some error");
    storage.save_run(failed_run.clone()).await.unwrap();

    let scheduler = Scheduler::with_storage(Arc::clone(&storage));

    // Recover should not touch completed or failed runs
    let recovered = scheduler.recover().await.unwrap();
    assert_eq!(recovered.len(), 0);

    // Verify statuses unchanged
    let check_completed = storage.get_run(&completed_run.id).await.unwrap();
    let check_failed = storage.get_run(&failed_run.id).await.unwrap();

    assert_eq!(check_completed.status, RunStatus::Completed);
    assert_eq!(check_failed.status, RunStatus::Failed);
}

/// Test: Recovery handles empty storage gracefully.
#[tokio::test]
async fn test_recovery_with_empty_storage() {
    let storage = InMemoryStorage::new();
    let scheduler = Scheduler::new(storage);

    let recovered = scheduler.recover().await.unwrap();
    assert_eq!(recovered.len(), 0);
}

/// Test: Job can be re-triggered after previous run failed.
#[tokio::test]
async fn test_retrigger_after_failure() {
    let storage = InMemoryStorage::new();
    let mut scheduler = Scheduler::new(storage);

    let dag = DagBuilder::new("dag", "Test DAG")
        .add_task(SuccessTask::new("task"))
        .build()
        .unwrap();

    let job = Job::new("retrigger_job", "Retrigger Job", dag);
    scheduler.register(job);

    let (handle, task) = scheduler.start().await;

    // First trigger
    let run1 = handle.trigger("retrigger_job").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Second trigger (should work even though first completed)
    let run2 = handle.trigger("retrigger_job").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Both runs should have unique IDs
    assert_ne!(run1, run2);

    handle.shutdown().await.unwrap();
    let _ = task.await;
}

/// Test: Scheduler state transitions.
#[tokio::test]
async fn test_scheduler_state_transitions() {
    let storage = InMemoryStorage::new();
    let scheduler = Scheduler::new(storage);

    let (handle, task) = scheduler.start().await;

    // Initially running
    assert!(handle.is_running().await);
    assert!(!handle.is_paused().await);

    // Pause
    handle.pause().await.unwrap();
    assert!(handle.is_paused().await);
    assert!(!handle.is_running().await);

    // Resume
    handle.resume().await.unwrap();
    assert!(handle.is_running().await);
    assert!(!handle.is_paused().await);

    // Shutdown
    handle.shutdown().await.unwrap();
    let state = handle.state().await;
    assert_eq!(state, petit::SchedulerState::Stopped);

    let _ = task.await;
}

/// Test: Manual triggers work while scheduler is paused.
#[tokio::test]
async fn test_manual_trigger_while_paused() {
    let storage = InMemoryStorage::new();
    let mut scheduler = Scheduler::new(storage);

    let dag = DagBuilder::new("dag", "Test DAG")
        .add_task(SuccessTask::new("task"))
        .build()
        .unwrap();

    let job = Job::new("manual_job", "Manual Job", dag);
    scheduler.register(job);

    let (handle, task) = scheduler.start().await;

    // Pause the scheduler
    handle.pause().await.unwrap();
    assert!(handle.is_paused().await);

    // Manual trigger should still work
    let run_id = handle.trigger("manual_job").await.unwrap();
    assert!(!run_id.as_uuid().is_nil());

    tokio::time::sleep(Duration::from_millis(100)).await;

    handle.shutdown().await.unwrap();
    let _ = task.await;
}

/// Test: Run status transitions are tracked correctly.
#[tokio::test]
async fn test_run_status_transitions() {
    let storage = Arc::new(InMemoryStorage::new());
    let mut scheduler = Scheduler::with_storage(Arc::clone(&storage));

    let dag = DagBuilder::new("dag", "Test DAG")
        .add_task(SuccessTask::new("task"))
        .build()
        .unwrap();

    let job = Job::new("status_job", "Status Job", dag);
    scheduler.register(job);

    let (handle, task) = scheduler.start().await;

    // Trigger job
    let run_id = handle.trigger("status_job").await.unwrap();

    // Wait for completion
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Check final status
    let run = storage.get_run(&run_id).await.unwrap();
    assert_eq!(run.status, RunStatus::Completed);
    assert!(run.ended_at.is_some());
    assert!(run.duration.is_some());

    handle.shutdown().await.unwrap();
    let _ = task.await;
}

/// Test: Failed job run has error message.
#[tokio::test]
async fn test_failed_run_has_error() {
    /// Task that always fails.
    struct FailTask;

    #[async_trait]
    impl Task for FailTask {
        fn name(&self) -> &str {
            "fail_task"
        }

        async fn execute(&self, _ctx: &mut TaskContext) -> Result<(), TaskError> {
            Err(TaskError::ExecutionFailed("intentional failure".into()))
        }
    }

    let storage = Arc::new(InMemoryStorage::new());
    let mut scheduler = Scheduler::with_storage(Arc::clone(&storage));

    let dag = DagBuilder::new("dag", "Test DAG")
        .add_task(Arc::new(FailTask) as Arc<dyn Task>)
        .build()
        .unwrap();

    let job = Job::new("fail_job", "Fail Job", dag);
    scheduler.register(job);

    let (handle, task) = scheduler.start().await;

    let run_id = handle.trigger("fail_job").await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    let run = storage.get_run(&run_id).await.unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert!(run.error.is_some());

    handle.shutdown().await.unwrap();
    let _ = task.await;
}

/// Test: Multiple recovery cycles don't duplicate marks.
#[tokio::test]
async fn test_multiple_recovery_cycles() {
    let storage = Arc::new(InMemoryStorage::new());

    // Create an interrupted run
    let mut run = StoredRun::new(RunId::new(), JobId::new("job1"));
    run.mark_running();
    storage.save_run(run.clone()).await.unwrap();

    // First recovery
    let scheduler1 = Scheduler::with_storage(Arc::clone(&storage));
    let recovered1 = scheduler1.recover().await.unwrap();
    assert_eq!(recovered1.len(), 1);

    // Second recovery should find nothing (already marked)
    let scheduler2 = Scheduler::with_storage(Arc::clone(&storage));
    let recovered2 = scheduler2.recover().await.unwrap();
    assert_eq!(recovered2.len(), 0);

    // Status should still be interrupted
    let final_run = storage.get_run(&run.id).await.unwrap();
    assert_eq!(final_run.status, RunStatus::Interrupted);
}
