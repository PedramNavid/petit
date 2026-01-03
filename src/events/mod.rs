//! Lifecycle events and event handling.
//!
//! This module provides event emission for task and job lifecycle events,
//! enabling observability into DAG execution.

use async_trait::async_trait;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::core::types::{DagId, JobId, RunId, TaskId};

/// Lifecycle events emitted during execution.
#[derive(Debug, Clone)]
pub enum Event {
    /// A task has started execution.
    TaskStarted {
        task_id: TaskId,
        dag_id: DagId,
        timestamp: Instant,
    },

    /// A task completed successfully.
    TaskCompleted {
        task_id: TaskId,
        dag_id: DagId,
        duration: Duration,
        stdout: Option<String>,
        stderr: Option<String>,
        exit_code: Option<i32>,
        timestamp: Instant,
    },

    /// A task failed with an error.
    TaskFailed {
        task_id: TaskId,
        dag_id: DagId,
        error: String,
        stdout: Option<String>,
        stderr: Option<String>,
        exit_code: Option<i32>,
        timestamp: Instant,
    },

    /// A task is being retried after failure.
    ///
    /// This event is emitted immediately before the retry delay, allowing
    /// consumers to observe retries in real-time.
    ///
    /// Note: `max_attempts` represents the total number of attempts including
    /// the initial attempt (i.e., `RetryPolicy.max_attempts + 1`). For example,
    /// if `RetryPolicy.max_attempts = 3` (meaning 3 retries), then `max_attempts`
    /// in this event will be 4 (1 initial + 3 retries).
    TaskRetrying {
        task_id: TaskId,
        dag_id: DagId,
        /// The attempt number that just failed (1-indexed).
        /// For example, attempt=1 means the initial attempt failed.
        attempt: u32,
        /// Total number of attempts that will be made, including the initial attempt.
        max_attempts: u32,
        timestamp: Instant,
    },

    /// A job has started execution.
    JobStarted {
        job_id: JobId,
        run_id: RunId,
        timestamp: Instant,
    },

    /// A job completed (successfully or with failures).
    JobCompleted {
        job_id: JobId,
        run_id: RunId,
        success: bool,
        duration: Duration,
        timestamp: Instant,
    },
}

impl Event {
    /// Get the timestamp of the event.
    pub fn timestamp(&self) -> Instant {
        match self {
            Event::TaskStarted { timestamp, .. } => *timestamp,
            Event::TaskCompleted { timestamp, .. } => *timestamp,
            Event::TaskFailed { timestamp, .. } => *timestamp,
            Event::TaskRetrying { timestamp, .. } => *timestamp,
            Event::JobStarted { timestamp, .. } => *timestamp,
            Event::JobCompleted { timestamp, .. } => *timestamp,
        }
    }

    /// Create a TaskStarted event.
    pub fn task_started(task_id: TaskId, dag_id: DagId) -> Self {
        Event::TaskStarted {
            task_id,
            dag_id,
            timestamp: Instant::now(),
        }
    }

    /// Create a TaskCompleted event.
    pub fn task_completed(task_id: TaskId, dag_id: DagId, duration: Duration) -> Self {
        Event::TaskCompleted {
            task_id,
            dag_id,
            duration,
            stdout: None,
            stderr: None,
            exit_code: None,
            timestamp: Instant::now(),
        }
    }

    /// Create a TaskCompleted event with output details.
    pub fn task_completed_with_output(
        task_id: TaskId,
        dag_id: DagId,
        duration: Duration,
        stdout: Option<String>,
        stderr: Option<String>,
        exit_code: Option<i32>,
    ) -> Self {
        Event::TaskCompleted {
            task_id,
            dag_id,
            duration,
            stdout,
            stderr,
            exit_code,
            timestamp: Instant::now(),
        }
    }

    /// Create a TaskFailed event.
    pub fn task_failed(task_id: TaskId, dag_id: DagId, error: String) -> Self {
        Event::TaskFailed {
            task_id,
            dag_id,
            error,
            stdout: None,
            stderr: None,
            exit_code: None,
            timestamp: Instant::now(),
        }
    }

    /// Create a TaskFailed event with output details.
    pub fn task_failed_with_output(
        task_id: TaskId,
        dag_id: DagId,
        error: String,
        stdout: Option<String>,
        stderr: Option<String>,
        exit_code: Option<i32>,
    ) -> Self {
        Event::TaskFailed {
            task_id,
            dag_id,
            error,
            stdout,
            stderr,
            exit_code,
            timestamp: Instant::now(),
        }
    }

    /// Create a TaskRetrying event.
    pub fn task_retrying(task_id: TaskId, dag_id: DagId, attempt: u32, max_attempts: u32) -> Self {
        Event::TaskRetrying {
            task_id,
            dag_id,
            attempt,
            max_attempts,
            timestamp: Instant::now(),
        }
    }

    /// Create a JobStarted event.
    pub fn job_started(job_id: JobId, run_id: RunId) -> Self {
        Event::JobStarted {
            job_id,
            run_id,
            timestamp: Instant::now(),
        }
    }

    /// Create a JobCompleted event.
    pub fn job_completed(job_id: JobId, run_id: RunId, success: bool, duration: Duration) -> Self {
        Event::JobCompleted {
            job_id,
            run_id,
            success,
            duration,
            timestamp: Instant::now(),
        }
    }
}

/// Handler for receiving lifecycle events.
#[async_trait]
pub trait EventHandler: Send + Sync {
    /// Handle an event.
    async fn handle(&self, event: &Event);
}

/// Event bus for distributing events to registered handlers.
pub struct EventBus {
    handlers: RwLock<Vec<Arc<dyn EventHandler>>>,
}

impl EventBus {
    /// Create a new event bus with no handlers.
    pub fn new() -> Self {
        Self {
            handlers: RwLock::new(Vec::new()),
        }
    }

    /// Register an event handler.
    pub async fn register(&self, handler: Arc<dyn EventHandler>) {
        let mut handlers = self.handlers.write().await;
        handlers.push(handler);
    }

    /// Emit an event to all registered handlers.
    pub async fn emit(&self, event: Event) {
        let handlers = self.handlers.read().await;
        for handler in handlers.iter() {
            handler.handle(&event).await;
        }
    }

    /// Get the number of registered handlers.
    pub async fn handler_count(&self) -> usize {
        self.handlers.read().await.len()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use tokio::sync::Mutex;

    /// Test handler that records received events.
    struct RecordingHandler {
        events: Mutex<Vec<Event>>,
    }

    impl RecordingHandler {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
            }
        }

        async fn events(&self) -> Vec<Event> {
            self.events.lock().await.clone()
        }
    }

    #[async_trait]
    impl EventHandler for RecordingHandler {
        async fn handle(&self, event: &Event) {
            self.events.lock().await.push(event.clone());
        }
    }

    /// Test handler that counts events.
    struct CountingHandler {
        count: AtomicU32,
    }

    impl CountingHandler {
        fn new() -> Self {
            Self {
                count: AtomicU32::new(0),
            }
        }

        fn count(&self) -> u32 {
            self.count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl EventHandler for CountingHandler {
        async fn handle(&self, _event: &Event) {
            self.count.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn test_emit_task_started_event() {
        let handler = Arc::new(RecordingHandler::new());
        let bus = EventBus::new();
        bus.register(handler.clone()).await;

        let event = Event::task_started(TaskId::new("extract"), DagId::new("etl"));
        bus.emit(event).await;

        let events = handler.events().await;
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::TaskStarted {
                task_id, dag_id, ..
            } => {
                assert_eq!(task_id.as_str(), "extract");
                assert_eq!(dag_id.as_str(), "etl");
            }
            _ => panic!("Expected TaskStarted event"),
        }
    }

    #[tokio::test]
    async fn test_emit_task_completed_event_with_duration() {
        let handler = Arc::new(RecordingHandler::new());
        let bus = EventBus::new();
        bus.register(handler.clone()).await;

        let duration = Duration::from_millis(150);
        let event = Event::task_completed(TaskId::new("transform"), DagId::new("etl"), duration);
        bus.emit(event).await;

        let events = handler.events().await;
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::TaskCompleted {
                task_id,
                duration: d,
                ..
            } => {
                assert_eq!(task_id.as_str(), "transform");
                assert_eq!(*d, Duration::from_millis(150));
            }
            _ => panic!("Expected TaskCompleted event"),
        }
    }

    #[tokio::test]
    async fn test_emit_task_failed_event_with_error() {
        let handler = Arc::new(RecordingHandler::new());
        let bus = EventBus::new();
        bus.register(handler.clone()).await;

        let event = Event::task_failed(
            TaskId::new("load"),
            DagId::new("etl"),
            "connection refused".to_string(),
        );
        bus.emit(event).await;

        let events = handler.events().await;
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::TaskFailed { task_id, error, .. } => {
                assert_eq!(task_id.as_str(), "load");
                assert_eq!(error, "connection refused");
            }
            _ => panic!("Expected TaskFailed event"),
        }
    }

    #[tokio::test]
    async fn test_emit_task_retrying_event_with_attempt_count() {
        let handler = Arc::new(RecordingHandler::new());
        let bus = EventBus::new();
        bus.register(handler.clone()).await;

        let event = Event::task_retrying(TaskId::new("flaky"), DagId::new("dag"), 2, 5);
        bus.emit(event).await;

        let events = handler.events().await;
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::TaskRetrying {
                task_id,
                attempt,
                max_attempts,
                ..
            } => {
                assert_eq!(task_id.as_str(), "flaky");
                assert_eq!(*attempt, 2);
                assert_eq!(*max_attempts, 5);
            }
            _ => panic!("Expected TaskRetrying event"),
        }
    }

    #[tokio::test]
    async fn test_emit_job_started_event() {
        let handler = Arc::new(RecordingHandler::new());
        let bus = EventBus::new();
        bus.register(handler.clone()).await;

        let run_id = RunId::new();
        let expected_uuid = *run_id.as_uuid();
        let event = Event::job_started(JobId::new("daily_etl"), run_id);
        bus.emit(event).await;

        let events = handler.events().await;
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::JobStarted { job_id, run_id, .. } => {
                assert_eq!(job_id.as_str(), "daily_etl");
                assert_eq!(*run_id.as_uuid(), expected_uuid);
            }
            _ => panic!("Expected JobStarted event"),
        }
    }

    #[tokio::test]
    async fn test_emit_job_completed_event() {
        let handler = Arc::new(RecordingHandler::new());
        let bus = EventBus::new();
        bus.register(handler.clone()).await;

        let duration = Duration::from_secs(30);
        let run_id = RunId::new();
        let expected_uuid = *run_id.as_uuid();
        let event = Event::job_completed(JobId::new("daily_etl"), run_id, true, duration);
        bus.emit(event).await;

        let events = handler.events().await;
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::JobCompleted {
                job_id,
                run_id,
                success,
                duration: d,
                ..
            } => {
                assert_eq!(job_id.as_str(), "daily_etl");
                assert_eq!(*run_id.as_uuid(), expected_uuid);
                assert!(*success);
                assert_eq!(*d, Duration::from_secs(30));
            }
            _ => panic!("Expected JobCompleted event"),
        }
    }

    #[tokio::test]
    async fn test_register_event_handler() {
        let bus = EventBus::new();
        assert_eq!(bus.handler_count().await, 0);

        let handler = Arc::new(CountingHandler::new());
        bus.register(handler).await;
        assert_eq!(bus.handler_count().await, 1);
    }

    #[tokio::test]
    async fn test_multiple_handlers_receive_same_event() {
        let handler1 = Arc::new(CountingHandler::new());
        let handler2 = Arc::new(CountingHandler::new());
        let handler3 = Arc::new(CountingHandler::new());

        let bus = EventBus::new();
        bus.register(handler1.clone()).await;
        bus.register(handler2.clone()).await;
        bus.register(handler3.clone()).await;

        let event = Event::task_started(TaskId::new("test"), DagId::new("dag"));
        bus.emit(event).await;

        assert_eq!(handler1.count(), 1);
        assert_eq!(handler2.count(), 1);
        assert_eq!(handler3.count(), 1);
    }

    #[tokio::test]
    async fn test_event_timestamps_are_accurate() {
        let before = Instant::now();
        let event = Event::task_started(TaskId::new("test"), DagId::new("dag"));
        let after = Instant::now();

        let timestamp = event.timestamp();
        assert!(timestamp >= before);
        assert!(timestamp <= after);
    }

    #[tokio::test]
    async fn test_multiple_events_in_sequence() {
        let handler = Arc::new(RecordingHandler::new());
        let bus = EventBus::new();
        bus.register(handler.clone()).await;

        // Emit a sequence of events
        bus.emit(Event::task_started(TaskId::new("t1"), DagId::new("dag")))
            .await;
        bus.emit(Event::task_completed(
            TaskId::new("t1"),
            DagId::new("dag"),
            Duration::from_millis(100),
        ))
        .await;
        bus.emit(Event::task_started(TaskId::new("t2"), DagId::new("dag")))
            .await;
        bus.emit(Event::task_failed(
            TaskId::new("t2"),
            DagId::new("dag"),
            "oops".to_string(),
        ))
        .await;

        let events = handler.events().await;
        assert_eq!(events.len(), 4);

        // Verify order and types
        assert!(matches!(events[0], Event::TaskStarted { .. }));
        assert!(matches!(events[1], Event::TaskCompleted { .. }));
        assert!(matches!(events[2], Event::TaskStarted { .. }));
        assert!(matches!(events[3], Event::TaskFailed { .. }));
    }

    #[tokio::test]
    async fn test_no_handlers_does_not_panic() {
        let bus = EventBus::new();
        // Should not panic even with no handlers
        bus.emit(Event::task_started(TaskId::new("test"), DagId::new("dag")))
            .await;
    }

    #[tokio::test]
    async fn test_task_completed_with_output() {
        let handler = Arc::new(RecordingHandler::new());
        let bus = EventBus::new();
        bus.register(handler.clone()).await;

        let event = Event::task_completed_with_output(
            TaskId::new("echo_task"),
            DagId::new("pipeline"),
            Duration::from_millis(50),
            Some("Hello, World!".to_string()),
            Some("warning: deprecated".to_string()),
            Some(0),
        );
        bus.emit(event).await;

        let events = handler.events().await;
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::TaskCompleted {
                task_id,
                stdout,
                stderr,
                exit_code,
                ..
            } => {
                assert_eq!(task_id.as_str(), "echo_task");
                assert_eq!(stdout.as_deref(), Some("Hello, World!"));
                assert_eq!(stderr.as_deref(), Some("warning: deprecated"));
                assert_eq!(*exit_code, Some(0));
            }
            _ => panic!("Expected TaskCompleted event"),
        }
    }

    #[tokio::test]
    async fn test_task_failed_with_output() {
        let handler = Arc::new(RecordingHandler::new());
        let bus = EventBus::new();
        bus.register(handler.clone()).await;

        let event = Event::task_failed_with_output(
            TaskId::new("failing_task"),
            DagId::new("pipeline"),
            "command failed".to_string(),
            Some("partial output".to_string()),
            Some("error: file not found".to_string()),
            Some(1),
        );
        bus.emit(event).await;

        let events = handler.events().await;
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::TaskFailed {
                task_id,
                error,
                stdout,
                stderr,
                exit_code,
                ..
            } => {
                assert_eq!(task_id.as_str(), "failing_task");
                assert_eq!(error, "command failed");
                assert_eq!(stdout.as_deref(), Some("partial output"));
                assert_eq!(stderr.as_deref(), Some("error: file not found"));
                assert_eq!(*exit_code, Some(1));
            }
            _ => panic!("Expected TaskFailed event"),
        }
    }

    #[tokio::test]
    async fn test_task_completed_without_output() {
        // Verify that the basic constructor still works with None values
        let event = Event::task_completed(
            TaskId::new("task"),
            DagId::new("dag"),
            Duration::from_millis(100),
        );

        match event {
            Event::TaskCompleted {
                stdout,
                stderr,
                exit_code,
                ..
            } => {
                assert!(stdout.is_none());
                assert!(stderr.is_none());
                assert!(exit_code.is_none());
            }
            _ => panic!("Expected TaskCompleted event"),
        }
    }

    #[tokio::test]
    async fn test_task_failed_without_output() {
        // Verify that the basic constructor still works with None values
        let event = Event::task_failed(TaskId::new("task"), DagId::new("dag"), "error".to_string());

        match event {
            Event::TaskFailed {
                stdout,
                stderr,
                exit_code,
                ..
            } => {
                assert!(stdout.is_none());
                assert!(stderr.is_none());
                assert!(exit_code.is_none());
            }
            _ => panic!("Expected TaskFailed event"),
        }
    }
}
