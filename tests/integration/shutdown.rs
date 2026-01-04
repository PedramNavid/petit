//! Graceful shutdown integration tests.
//!
//! Tests that verify the scheduler gracefully handles shutdown by
//! waiting for running jobs to complete before exiting.

use async_trait::async_trait;
use petit::{
    DagBuilder, Event, EventBus, EventHandler, InMemoryStorage, Job, RunStatus, Scheduler, Storage,
    Task, TaskContext, TaskError,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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

    async fn job_completed_count(&self) -> usize {
        self.events
            .lock()
            .await
            .iter()
            .filter(|e| matches!(e, Event::JobCompleted { .. }))
            .count()
    }

    async fn has_successful_completion(&self) -> bool {
        self.events
            .lock()
            .await
            .iter()
            .any(|e| matches!(e, Event::JobCompleted { success, .. } if *success))
    }
}

#[async_trait]
impl EventHandler for RecordingHandler {
    async fn handle(&self, event: &Event) {
        self.events.lock().await.push(event.clone());
    }
}

/// Task that takes a specified duration to complete.
struct SlowTask {
    name: String,
    duration: Duration,
    started: AtomicBool,
    completed: AtomicBool,
}

impl SlowTask {
    fn new(name: &str, duration: Duration) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            duration,
            started: AtomicBool::new(false),
            completed: AtomicBool::new(false),
        })
    }

    fn was_started(&self) -> bool {
        self.started.load(Ordering::SeqCst)
    }

    fn was_completed(&self) -> bool {
        self.completed.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Task for SlowTask {
    fn name(&self) -> &str {
        &self.name
    }

    async fn execute(&self, _ctx: &mut TaskContext) -> Result<(), TaskError> {
        self.started.store(true, Ordering::SeqCst);
        tokio::time::sleep(self.duration).await;
        self.completed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

/// Test: Graceful shutdown waits for running jobs to complete.
///
/// This test verifies that when shutdown is triggered while a job is running,
/// the scheduler waits for that job to complete before returning from shutdown.
#[tokio::test]
async fn test_graceful_shutdown_waits_for_jobs() {
    let storage = Arc::new(InMemoryStorage::new());
    let event_bus = EventBus::new();
    let handler = RecordingHandler::new();
    event_bus.register(handler.clone()).await;

    // Create a slow task that takes 300ms to complete
    let slow_task = SlowTask::new("slow_task", Duration::from_millis(300));
    let task_ref = slow_task.clone();

    let dag = DagBuilder::new("slow_dag", "Slow DAG")
        .add_task(slow_task as Arc<dyn Task>)
        .build()
        .unwrap();

    let job = Job::new("slow_job", "Slow Job", dag);

    let mut scheduler = Scheduler::with_storage(Arc::clone(&storage))
        .with_event_bus(event_bus)
        .with_shutdown_timeout(Duration::from_secs(5)); // Generous timeout

    scheduler.register(job);

    let (handle, scheduler_task) = scheduler.start().await;

    // Trigger the slow job
    let run_id = handle.trigger("slow_job").await.unwrap();

    // Give the job time to start but not complete
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Verify the task has started
    assert!(task_ref.was_started(), "Task should have started");
    assert!(
        !task_ref.was_completed(),
        "Task should not have completed yet"
    );

    // Trigger shutdown while job is running
    let shutdown_start = std::time::Instant::now();
    handle.shutdown().await.unwrap();
    let shutdown_duration = shutdown_start.elapsed();

    // Wait for the scheduler task to finish
    let _ = scheduler_task.await;

    // Verify the task completed (graceful shutdown waited for it)
    assert!(
        task_ref.was_completed(),
        "Task should have completed during graceful shutdown"
    );

    // Shutdown should have taken at least as long as the remaining task duration
    // (approximately 250ms remaining after the 50ms sleep)
    assert!(
        shutdown_duration >= Duration::from_millis(200),
        "Shutdown should have waited for the job to complete. Duration: {:?}",
        shutdown_duration
    );

    // Verify job completed successfully
    assert_eq!(
        handler.job_completed_count().await,
        1,
        "Job should have completed"
    );
    assert!(
        handler.has_successful_completion().await,
        "Job should have completed successfully"
    );

    // Verify run status in storage
    let run = storage.get_run(&run_id).await.unwrap();
    assert_eq!(
        run.status,
        RunStatus::Completed,
        "Run should be marked as completed"
    );
}

/// Test: Graceful shutdown times out when jobs run too long.
///
/// This test verifies that when a job runs longer than the shutdown timeout,
/// the scheduler eventually proceeds with shutdown (logging a warning).
#[tokio::test]
async fn test_graceful_shutdown_timeout_exceeded() {
    let storage = Arc::new(InMemoryStorage::new());

    // Create a very slow task that takes 2 seconds
    let slow_task = SlowTask::new("very_slow_task", Duration::from_secs(2));
    let task_ref = slow_task.clone();

    let dag = DagBuilder::new("very_slow_dag", "Very Slow DAG")
        .add_task(slow_task as Arc<dyn Task>)
        .build()
        .unwrap();

    let job = Job::new("very_slow_job", "Very Slow Job", dag);

    // Set a short shutdown timeout (200ms) that will be exceeded
    let mut scheduler = Scheduler::with_storage(Arc::clone(&storage))
        .with_shutdown_timeout(Duration::from_millis(200));

    scheduler.register(job);

    let (handle, scheduler_task) = scheduler.start().await;

    // Trigger the slow job
    let run_id = handle.trigger("very_slow_job").await.unwrap();

    // Give the job time to start
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(task_ref.was_started(), "Task should have started");

    // Trigger shutdown - it should timeout rather than wait for the full 2 seconds
    let shutdown_start = std::time::Instant::now();
    handle.shutdown().await.unwrap();
    let shutdown_duration = shutdown_start.elapsed();

    // Wait for the scheduler task to finish
    let _ = scheduler_task.await;

    // Shutdown should have completed after the timeout (~200ms), not after the task (~2s)
    // Allow some slack for timing variations
    assert!(
        shutdown_duration < Duration::from_millis(500),
        "Shutdown should timeout, not wait for full job. Duration: {:?}",
        shutdown_duration
    );

    // The task may or may not have completed (it was still running when we timed out)
    // The important thing is that shutdown didn't hang

    // Note: The run status may be Running since the job was cut off
    // This is expected behavior when the timeout is exceeded
    let run = storage.get_run(&run_id).await.unwrap();
    assert!(
        matches!(run.status, RunStatus::Running | RunStatus::Completed),
        "Run status should be Running (job cut off) or Completed. Got: {:?}",
        run.status
    );
}

/// Test: Graceful shutdown with no running jobs completes immediately.
///
/// This test verifies that shutdown completes quickly when there are no
/// running jobs to wait for.
#[tokio::test]
async fn test_graceful_shutdown_with_no_running_jobs() {
    let storage = InMemoryStorage::new();
    let scheduler = Scheduler::new(storage).with_shutdown_timeout(Duration::from_secs(5));

    let (handle, scheduler_task) = scheduler.start().await;

    // Don't trigger any jobs - just shutdown immediately
    let shutdown_start = std::time::Instant::now();
    handle.shutdown().await.unwrap();
    let shutdown_duration = shutdown_start.elapsed();

    // Wait for the scheduler task to finish
    let _ = scheduler_task.await;

    // Shutdown should be very fast (well under 100ms) since there's nothing to wait for
    assert!(
        shutdown_duration < Duration::from_millis(100),
        "Shutdown with no running jobs should be fast. Duration: {:?}",
        shutdown_duration
    );
}

/// Test: Graceful shutdown waits for multiple concurrent jobs.
///
/// This test verifies that shutdown waits for all running jobs, not just one.
#[tokio::test]
async fn test_graceful_shutdown_waits_for_multiple_jobs() {
    let storage = Arc::new(InMemoryStorage::new());
    let event_bus = EventBus::new();
    let handler = RecordingHandler::new();
    event_bus.register(handler.clone()).await;

    // Create multiple slow tasks with different durations
    let slow_task1 = SlowTask::new("slow_task_1", Duration::from_millis(200));
    let slow_task2 = SlowTask::new("slow_task_2", Duration::from_millis(300));
    let task1_ref = slow_task1.clone();
    let task2_ref = slow_task2.clone();

    let dag1 = DagBuilder::new("slow_dag_1", "Slow DAG 1")
        .add_task(slow_task1 as Arc<dyn Task>)
        .build()
        .unwrap();

    let dag2 = DagBuilder::new("slow_dag_2", "Slow DAG 2")
        .add_task(slow_task2 as Arc<dyn Task>)
        .build()
        .unwrap();

    let job1 = Job::new("slow_job_1", "Slow Job 1", dag1);
    let job2 = Job::new("slow_job_2", "Slow Job 2", dag2);

    let mut scheduler = Scheduler::with_storage(Arc::clone(&storage))
        .with_event_bus(event_bus)
        .with_shutdown_timeout(Duration::from_secs(5))
        .with_max_concurrent_jobs(5);

    scheduler.register(job1);
    scheduler.register(job2);

    let (handle, scheduler_task) = scheduler.start().await;

    // Trigger both jobs
    let _run_id1 = handle.trigger("slow_job_1").await.unwrap();
    let _run_id2 = handle.trigger("slow_job_2").await.unwrap();

    // Give the jobs time to start
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(task1_ref.was_started(), "Task 1 should have started");
    assert!(task2_ref.was_started(), "Task 2 should have started");

    // Trigger shutdown
    let shutdown_start = std::time::Instant::now();
    handle.shutdown().await.unwrap();
    let shutdown_duration = shutdown_start.elapsed();

    // Wait for the scheduler task to finish
    let _ = scheduler_task.await;

    // Both tasks should have completed
    assert!(task1_ref.was_completed(), "Task 1 should have completed");
    assert!(task2_ref.was_completed(), "Task 2 should have completed");

    // Shutdown should have waited for the slowest job (~300ms - 50ms = 250ms remaining)
    assert!(
        shutdown_duration >= Duration::from_millis(200),
        "Shutdown should wait for slowest job. Duration: {:?}",
        shutdown_duration
    );

    // Both jobs should have completed
    assert_eq!(
        handler.job_completed_count().await,
        2,
        "Both jobs should have completed"
    );
}

/// Test: Jobs triggered during shutdown wait are still completed.
///
/// This test verifies that even if new jobs are triggered just before shutdown,
/// they are allowed to complete during the graceful shutdown period.
#[tokio::test]
async fn test_graceful_shutdown_completes_recently_triggered_job() {
    let storage = Arc::new(InMemoryStorage::new());
    let event_bus = EventBus::new();
    let handler = RecordingHandler::new();
    event_bus.register(handler.clone()).await;

    let slow_task = SlowTask::new("slow_task", Duration::from_millis(200));
    let task_ref = slow_task.clone();

    let dag = DagBuilder::new("slow_dag", "Slow DAG")
        .add_task(slow_task as Arc<dyn Task>)
        .build()
        .unwrap();

    let job = Job::new("slow_job", "Slow Job", dag);

    let mut scheduler = Scheduler::with_storage(Arc::clone(&storage))
        .with_event_bus(event_bus)
        .with_shutdown_timeout(Duration::from_secs(5));

    scheduler.register(job);

    let (handle, scheduler_task) = scheduler.start().await;

    // Trigger the job and immediately start shutdown
    let run_id = handle.trigger("slow_job").await.unwrap();

    // Very short delay - job is barely started
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Trigger shutdown almost immediately after triggering the job
    handle.shutdown().await.unwrap();

    // Wait for the scheduler task to finish
    let _ = scheduler_task.await;

    // The task should still have completed
    assert!(
        task_ref.was_completed(),
        "Task should complete during graceful shutdown"
    );

    // Verify job completed successfully
    let run = storage.get_run(&run_id).await.unwrap();
    assert_eq!(
        run.status,
        RunStatus::Completed,
        "Run should be marked as completed"
    );
}
