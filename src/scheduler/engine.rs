//! Scheduler engine implementation.
//!
//! The scheduler is responsible for:
//! - Triggering jobs at scheduled times
//! - Respecting cross-job dependencies
//! - Manual job triggers
//! - Pause and resume functionality
//! - Recovery from interruptions
//! - Event emission

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::{RwLock, mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::core::context::TaskContext;
use crate::core::job::{DependencyCondition, Job};
use crate::core::types::{JobId, RunId, TaskId};
use crate::events::{Event, EventBus, EventHandler};
use crate::execution::DagExecutor;
use crate::storage::{RunStatus, Storage, StorageError, StoredJob, StoredRun, StoredTaskState};

/// Buffer size for the command channel between SchedulerHandle and Scheduler.
const COMMAND_CHANNEL_BUFFER: usize = 32;

/// Number of most recent runs to check when evaluating job dependencies.
const DEPENDENCY_CHECK_LIMIT: usize = 1;

/// Event handler that updates task states in storage as tasks execute.
struct TaskStateUpdater<S: Storage> {
    storage: Arc<S>,
    run_id: RunId,
}

#[async_trait::async_trait]
impl<S: Storage + 'static> EventHandler for TaskStateUpdater<S> {
    async fn handle(&self, event: &Event) {
        match event {
            Event::TaskStarted { task_id, .. } => {
                if let Ok(mut state) = self.storage.get_task_state(&self.run_id, task_id).await {
                    state.mark_running();
                    if let Err(e) = self.storage.update_task_state(state).await {
                        tracing::warn!(task_id = %task_id, run_id = %self.run_id, error = %e, "Failed to update task state to running");
                    }
                }
            }
            Event::TaskCompleted { task_id, .. } => {
                if let Ok(mut state) = self.storage.get_task_state(&self.run_id, task_id).await {
                    state.mark_completed();
                    if let Err(e) = self.storage.update_task_state(state).await {
                        tracing::warn!(task_id = %task_id, run_id = %self.run_id, error = %e, "Failed to update task state to completed");
                    }
                }
            }
            Event::TaskFailed { task_id, error, .. } => {
                if let Ok(mut state) = self.storage.get_task_state(&self.run_id, task_id).await {
                    state.mark_failed(error);
                    if let Err(e) = self.storage.update_task_state(state).await {
                        tracing::warn!(task_id = %task_id, run_id = %self.run_id, error = %e, "Failed to update task state to failed");
                    }
                }
            }
            _ => {}
        }
    }
}

/// Event handler that forwards events to another event bus.
struct EventForwarder {
    target: Arc<EventBus>,
}

#[async_trait::async_trait]
impl EventHandler for EventForwarder {
    async fn handle(&self, event: &Event) {
        self.target.emit(event.clone()).await;
    }
}

/// Errors that can occur in the scheduler.
#[derive(Debug, Error)]
pub enum SchedulerError {
    /// Job not found.
    #[error("job not found: {0}")]
    JobNotFound(String),

    /// Storage error.
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    /// Channel error.
    #[error("channel error: {0}")]
    ChannelError(String),

    /// Job dependency not satisfied.
    #[error("job dependency not satisfied: {0}")]
    DependencyNotSatisfied(String),

    /// Max concurrent runs exceeded.
    #[error("max concurrent runs exceeded for job: {0}")]
    MaxConcurrentRunsExceeded(String),
}

/// State of the scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerState {
    /// Scheduler is stopped.
    Stopped,
    /// Scheduler is running.
    Running,
    /// Scheduler is paused.
    Paused,
}

/// Commands that can be sent to the scheduler.
enum SchedulerCommand {
    /// Trigger a job manually.
    Trigger {
        job_id: JobId,
        response: oneshot::Sender<Result<RunId, SchedulerError>>,
    },
    /// Pause the scheduler.
    Pause { response: oneshot::Sender<()> },
    /// Resume the scheduler.
    Resume { response: oneshot::Sender<()> },
    /// Shutdown the scheduler.
    Shutdown { response: oneshot::Sender<()> },
}

/// Handle for controlling the scheduler.
#[derive(Clone)]
pub struct SchedulerHandle {
    command_tx: mpsc::Sender<SchedulerCommand>,
    state: Arc<RwLock<SchedulerState>>,
}

impl SchedulerHandle {
    /// Helper to send a command that returns a result and wait for response.
    async fn send_result_command<T>(
        &self,
        build_command: impl FnOnce(oneshot::Sender<Result<T, SchedulerError>>) -> SchedulerCommand,
        operation: &str,
    ) -> Result<T, SchedulerError>
    where
        T: Send + 'static,
    {
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx
            .send(build_command(response_tx))
            .await
            .map_err(|_| {
                SchedulerError::ChannelError(format!("failed to send {} command", operation))
            })?;

        response_rx.await.map_err(|_| {
            SchedulerError::ChannelError(format!("failed to receive {} response", operation))
        })?
    }

    /// Helper to send a command that returns unit and wait for response.
    async fn send_unit_command(
        &self,
        build_command: impl FnOnce(oneshot::Sender<()>) -> SchedulerCommand,
        operation: &str,
    ) -> Result<(), SchedulerError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx
            .send(build_command(response_tx))
            .await
            .map_err(|_| {
                SchedulerError::ChannelError(format!("failed to send {} command", operation))
            })?;

        response_rx.await.map_err(|_| {
            SchedulerError::ChannelError(format!("failed to receive {} response", operation))
        })?;

        Ok(())
    }

    /// Trigger a job manually.
    pub async fn trigger(&self, job_id: impl Into<JobId>) -> Result<RunId, SchedulerError> {
        let job_id = job_id.into();
        self.send_result_command(
            |response| SchedulerCommand::Trigger { job_id, response },
            "trigger",
        )
        .await
    }

    /// Pause the scheduler.
    ///
    /// While paused, scheduled jobs will not be triggered, but manual triggers still work.
    pub async fn pause(&self) -> Result<(), SchedulerError> {
        self.send_unit_command(|response| SchedulerCommand::Pause { response }, "pause")
            .await
    }

    /// Resume the scheduler after being paused.
    pub async fn resume(&self) -> Result<(), SchedulerError> {
        self.send_unit_command(|response| SchedulerCommand::Resume { response }, "resume")
            .await
    }

    /// Shutdown the scheduler.
    pub async fn shutdown(&self) -> Result<(), SchedulerError> {
        self.send_unit_command(
            |response| SchedulerCommand::Shutdown { response },
            "shutdown",
        )
        .await
    }

    /// Get the current scheduler state.
    pub async fn state(&self) -> SchedulerState {
        *self.state.read().await
    }

    /// Check if the scheduler is running.
    pub async fn is_running(&self) -> bool {
        *self.state.read().await == SchedulerState::Running
    }

    /// Check if the scheduler is paused.
    pub async fn is_paused(&self) -> bool {
        *self.state.read().await == SchedulerState::Paused
    }
}

/// Main scheduler for job execution.
pub struct Scheduler<S: Storage> {
    /// Registered jobs.
    jobs: HashMap<JobId, Job>,
    /// Storage backend.
    storage: Arc<S>,
    /// Event bus for emitting events.
    event_bus: Arc<EventBus>,
    /// DAG executor for running jobs.
    dag_executor: Arc<DagExecutor>,
    /// Tick interval for checking schedules.
    tick_interval: Duration,
    /// Maximum concurrent jobs overall (None = unlimited).
    max_concurrent_jobs: Option<usize>,
    /// Currently running job handles mapped to (JobId, Handle).
    #[allow(clippy::type_complexity)]
    running_jobs: Arc<RwLock<HashMap<RunId, (JobId, JoinHandle<()>)>>>,
}

impl<S: Storage + 'static> Scheduler<S> {
    /// Create a new scheduler with the given storage.
    pub fn new(storage: S) -> Self {
        Self {
            jobs: HashMap::new(),
            storage: Arc::new(storage),
            event_bus: Arc::new(EventBus::new()),
            dag_executor: Arc::new(DagExecutor::default()),
            tick_interval: Duration::from_secs(1),
            max_concurrent_jobs: None,
            running_jobs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new scheduler with shared storage (for testing).
    pub fn with_storage(storage: Arc<S>) -> Self {
        Self {
            jobs: HashMap::new(),
            storage,
            event_bus: Arc::new(EventBus::new()),
            dag_executor: Arc::new(DagExecutor::default()),
            tick_interval: Duration::from_secs(1),
            max_concurrent_jobs: None,
            running_jobs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Set the event bus.
    pub fn with_event_bus(mut self, event_bus: EventBus) -> Self {
        self.event_bus = Arc::new(event_bus);
        self
    }

    /// Set the DAG executor.
    pub fn with_dag_executor(mut self, executor: DagExecutor) -> Self {
        self.dag_executor = Arc::new(executor);
        self
    }

    /// Set the tick interval.
    pub fn with_tick_interval(mut self, interval: Duration) -> Self {
        self.tick_interval = interval;
        self
    }

    /// Set the maximum concurrent jobs.
    pub fn with_max_concurrent_jobs(mut self, max: usize) -> Self {
        self.max_concurrent_jobs = Some(max);
        self
    }

    /// Register a job with the scheduler.
    pub fn register(&mut self, job: Job) {
        self.jobs.insert(job.id().clone(), job);
    }

    /// Get a registered job by ID.
    pub fn get_job(&self, id: &JobId) -> Option<&Job> {
        self.jobs.get(id)
    }

    /// List all registered jobs.
    pub fn jobs(&self) -> impl Iterator<Item = &Job> {
        self.jobs.values()
    }

    /// Get the event bus.
    pub fn event_bus(&self) -> &EventBus {
        &self.event_bus
    }

    /// Start the scheduler and return a handle for controlling it.
    pub async fn start(self) -> (SchedulerHandle, JoinHandle<()>) {
        // Sync job definitions to storage so TUI and other tools can see them
        self.sync_jobs_to_storage().await;

        let (command_tx, command_rx) = mpsc::channel(COMMAND_CHANNEL_BUFFER);
        let state = Arc::new(RwLock::new(SchedulerState::Running));

        let handle = SchedulerHandle {
            command_tx,
            state: Arc::clone(&state),
        };

        let scheduler_task = tokio::spawn(async move {
            self.run(command_rx, state).await;
        });

        (handle, scheduler_task)
    }

    /// Sync registered jobs to storage for visibility by TUI and other tools.
    ///
    /// This performs an upsert for all current jobs and removes any stale jobs
    /// that exist in storage but are no longer registered with the scheduler.
    async fn sync_jobs_to_storage(&self) {
        // Collect current job IDs
        let current_job_ids: std::collections::HashSet<_> = self.jobs.keys().cloned().collect();

        // Upsert all current jobs
        for job in self.jobs.values() {
            let mut stored = StoredJob::new(job.id().clone(), job.name(), job.dag().id().clone())
                .with_enabled(job.is_enabled());

            if let Some(schedule) = job.schedule() {
                stored = stored.with_schedule(schedule.expression());
            }

            if let Err(e) = self.storage.upsert_job(stored).await {
                tracing::warn!(job_id = %job.id(), error = %e, "Failed to sync job to storage");
            }
        }

        // Remove stale jobs from storage that are no longer registered
        match self.storage.list_jobs().await {
            Ok(stored_jobs) => {
                for stored_job in stored_jobs {
                    if !current_job_ids.contains(&stored_job.id) {
                        tracing::info!(job_id = %stored_job.id, "Removing stale job from storage");
                        if let Err(e) = self.storage.delete_job(&stored_job.id).await {
                            tracing::warn!(job_id = %stored_job.id, error = %e, "Failed to delete stale job");
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to list jobs for stale cleanup");
            }
        }
    }

    /// Recover interrupted runs from storage.
    ///
    /// For each incomplete run:
    /// - Marks the run as interrupted
    /// - Updates pending/running task states to failed ("Run was interrupted")
    /// - Leaves completed task states unchanged
    /// - Logs warnings for any task state update failures (non-blocking)
    pub async fn recover(&self) -> Result<Vec<RunId>, SchedulerError> {
        let incomplete_runs = self.storage.get_incomplete_runs().await?;
        let mut recovered = Vec::new();

        for run in incomplete_runs {
            // Mark the run as interrupted
            self.storage.mark_run_interrupted(&run.id).await?;

            // Mark all associated task states as failed to ensure consistency
            if let Ok(task_states) = self.storage.list_task_states(&run.id).await {
                for mut state in task_states {
                    // Only update task states that are still pending or running
                    if matches!(
                        state.status,
                        crate::storage::TaskRunStatus::Pending
                            | crate::storage::TaskRunStatus::Running
                    ) {
                        state.mark_failed("Run was interrupted");
                        if let Err(e) = self.storage.update_task_state(state.clone()).await {
                            tracing::warn!(
                                task_id = %state.task_id,
                                run_id = %run.id,
                                error = %e,
                                "Failed to update task state during recovery"
                            );
                        }
                    }
                }
            }

            recovered.push(run.id);
        }

        Ok(recovered)
    }

    /// Main scheduler loop.
    async fn run(
        self,
        mut command_rx: mpsc::Receiver<SchedulerCommand>,
        state: Arc<RwLock<SchedulerState>>,
    ) {
        let mut interval = tokio::time::interval(self.tick_interval);
        let mut last_check = chrono::Utc::now();

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let current_state = *state.read().await;
                    if current_state == SchedulerState::Running {
                        let now = chrono::Utc::now();
                        self.check_schedules(last_check, now).await;
                        last_check = now;
                    }

                    // Clean up finished job handles
                    self.cleanup_finished_jobs().await;
                }

                Some(command) = command_rx.recv() => {
                    match command {
                        SchedulerCommand::Trigger { job_id, response } => {
                            let result = self.trigger_job(&job_id).await;
                            let _ = response.send(result);
                        }
                        SchedulerCommand::Pause { response } => {
                            let mut s = state.write().await;
                            *s = SchedulerState::Paused;
                            let _ = response.send(());
                        }
                        SchedulerCommand::Resume { response } => {
                            let mut s = state.write().await;
                            *s = SchedulerState::Running;
                            // Reset last_check to now to skip schedules that fired during pause.
                            // This prevents a burst of missed runs from being triggered on resume.
                            last_check = chrono::Utc::now();
                            tracing::info!("Scheduler resumed, skipping any schedules that fired during pause");
                            let _ = response.send(());
                        }
                        SchedulerCommand::Shutdown { response } => {
                            let mut s = state.write().await;
                            *s = SchedulerState::Stopped;
                            let _ = response.send(());
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Check all job schedules and trigger those that are due.
    async fn check_schedules(
        &self,
        last_check: chrono::DateTime<chrono::Utc>,
        now: chrono::DateTime<chrono::Utc>,
    ) {
        for job in self.jobs.values() {
            if !job.is_enabled() {
                continue;
            }

            if let Some(schedule) = job.schedule() {
                // Check if there's a scheduled occurrence between last_check and now
                if let Ok(next) = schedule.next_after(last_check) {
                    tracing::debug!(
                        job_id = %job.id(),
                        last_check = %last_check,
                        now = %now,
                        next = %next,
                        will_trigger = next <= now,
                        "Checking schedule"
                    );
                    // If the next occurrence is at or before now, trigger the job
                    if next <= now {
                        // Check dependencies before triggering
                        if self.check_dependencies(job).await {
                            tracing::info!(job_id = %job.id(), "Triggering scheduled job");
                            let _ = self.trigger_job(job.id()).await;
                        }
                    }
                }
            }
        }
    }

    /// Check if a job's dependencies are satisfied.
    async fn check_dependencies(&self, job: &Job) -> bool {
        for dep in job.dependencies() {
            let dep_job_id = dep.job_id();

            // Get the last run of the dependency job
            let runs = match self
                .storage
                .list_runs(dep_job_id, DEPENDENCY_CHECK_LIMIT)
                .await
            {
                Ok(runs) => runs,
                Err(_) => return false,
            };

            if runs.is_empty() {
                return false;
            }

            let last_run = &runs[0];

            match dep.condition() {
                DependencyCondition::LastSuccess => {
                    if last_run.status != RunStatus::Completed {
                        return false;
                    }
                }
                DependencyCondition::LastComplete => {
                    if !matches!(last_run.status, RunStatus::Completed | RunStatus::Failed) {
                        return false;
                    }
                }
                DependencyCondition::WithinWindow(window) => {
                    if last_run.status != RunStatus::Completed {
                        return false;
                    }

                    // Check if the run is within the window
                    if let Some(ended_at) = last_run.ended_at {
                        let now = std::time::SystemTime::now();
                        let elapsed = now.duration_since(ended_at).unwrap_or(Duration::MAX);
                        if elapsed > *window {
                            return false;
                        }
                    } else {
                        return false;
                    }
                }
            }
        }

        true
    }

    /// Trigger a job to run.
    async fn trigger_job(&self, job_id: &JobId) -> Result<RunId, SchedulerError> {
        let job = self
            .jobs
            .get(job_id)
            .ok_or_else(|| SchedulerError::JobNotFound(job_id.to_string()))?;

        // Check max concurrent runs for this job
        if let Some(max) = job.max_concurrency() {
            let running_count = self.count_running_for_job(job_id).await?;
            if running_count >= max {
                return Err(SchedulerError::MaxConcurrentRunsExceeded(
                    job_id.to_string(),
                ));
            }
        }

        // Check global max concurrent jobs
        if let Some(max) = self.max_concurrent_jobs {
            let running_count = self.running_jobs.read().await.len();
            if running_count >= max {
                return Err(SchedulerError::MaxConcurrentRunsExceeded(
                    "global limit".to_string(),
                ));
            }
        }

        // Check dependencies
        if !self.check_dependencies(job).await {
            return Err(SchedulerError::DependencyNotSatisfied(job_id.to_string()));
        }

        // Create a new run
        let run_id = RunId::new();
        let mut stored_run = StoredRun::new(run_id.clone(), job_id.clone());
        stored_run.mark_running();

        self.storage.save_run(stored_run).await?;

        // Emit JobStarted event
        self.event_bus
            .emit(Event::job_started(job_id.clone(), run_id.clone()))
            .await;

        // Spawn the job execution
        let job = job.clone();
        let storage = Arc::clone(&self.storage);
        let event_bus = Arc::clone(&self.event_bus);
        let dag_executor = Arc::clone(&self.dag_executor);
        let run_id_clone = run_id.clone();
        let job_id_clone = job_id.clone();
        let running_jobs = Arc::clone(&self.running_jobs);

        let handle = tokio::spawn(async move {
            let start = std::time::Instant::now();

            // Initialize task states
            for task_id in job.dag().task_ids() {
                let state = StoredTaskState::new(task_id.clone(), run_id_clone.clone());
                let _ = storage.save_task_state(state).await;
            }

            // Create a job-local event bus that:
            // 1. Updates task states in storage (local handler, dropped after job completes)
            // 2. Forwards events to the main event bus
            let job_event_bus = Arc::new(EventBus::new());
            let storage_handler = Arc::new(TaskStateUpdater {
                storage: Arc::clone(&storage),
                run_id: run_id_clone.clone(),
            });
            let forwarder = Arc::new(EventForwarder {
                target: Arc::clone(&event_bus),
            });
            job_event_bus.register(storage_handler).await;
            job_event_bus.register(forwarder).await;

            // Create context for execution
            let store = Arc::new(std::sync::RwLock::new(HashMap::new()));
            let config = Arc::new(job.config().clone());
            let mut ctx = TaskContext::new(store, TaskId::new("job"), config);

            // Execute the DAG with event emission (using job-local bus)
            let result = dag_executor
                .execute_with_events(job.dag(), &mut ctx, Some(job_event_bus))
                .await;

            // Update run status
            let duration = start.elapsed();
            if let Ok(mut run) = storage.get_run(&run_id_clone).await {
                if result.success {
                    run.mark_completed();
                } else {
                    run.mark_failed(format!("{} tasks failed", result.failed_count()));
                }
                let _ = storage.update_run(run).await;
            }

            // Emit JobCompleted event
            event_bus
                .emit(Event::job_completed(
                    job_id_clone,
                    run_id_clone.clone(),
                    result.success,
                    duration,
                ))
                .await;

            // Remove from running jobs
            running_jobs.write().await.remove(&run_id_clone);
        });

        // Track the running job
        self.running_jobs
            .write()
            .await
            .insert(run_id.clone(), (job_id.clone(), handle));

        Ok(run_id)
    }

    /// Count running instances of a specific job.
    async fn count_running_for_job(&self, job_id: &JobId) -> Result<usize, SchedulerError> {
        let running = self.running_jobs.read().await;
        Ok(running.values().filter(|(jid, _)| jid == job_id).count())
    }

    /// Clean up finished job handles.
    async fn cleanup_finished_jobs(&self) {
        let mut running = self.running_jobs.write().await;
        running.retain(|_, (_, handle)| !handle.is_finished());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::dag::DagBuilder;
    use crate::core::job::JobDependency;
    use crate::core::schedule::Schedule;
    use crate::core::task::{Task, TaskError};
    use crate::storage::InMemoryStorage;
    use async_trait::async_trait;
    use tokio::sync::Mutex;

    // Simple task that succeeds
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

        async fn execute(&self, _ctx: &mut TaskContext) -> Result<(), TaskError> {
            Ok(())
        }
    }

    // Recording event handler
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
    }

    #[async_trait]
    impl crate::events::EventHandler for RecordingHandler {
        async fn handle(&self, event: &Event) {
            self.events.lock().await.push(event.clone());
        }
    }

    fn create_simple_dag() -> crate::core::dag::Dag {
        DagBuilder::new("test_dag", "Test DAG")
            .add_task(SimpleTask::new("task1"))
            .build()
            .unwrap()
    }

    fn create_job(id: &str, name: &str) -> Job {
        Job::new(id, name, create_simple_dag())
    }

    fn create_scheduled_job(id: &str, name: &str, schedule: &str) -> Job {
        let sched = Schedule::new(schedule).unwrap();
        Job::new(id, name, create_simple_dag()).with_schedule(sched)
    }

    #[tokio::test]
    async fn test_scheduler_triggers_job_at_scheduled_time() {
        let storage = InMemoryStorage::new();
        let mut scheduler = Scheduler::new(storage).with_tick_interval(Duration::from_millis(50));

        // Job that runs every second
        let job = create_scheduled_job("frequent", "Frequent Job", "@every 1s");
        scheduler.register(job);

        let (handle, task) = scheduler.start().await;

        // Wait for at least one trigger
        tokio::time::sleep(Duration::from_millis(1200)).await;

        handle.shutdown().await.unwrap();
        let _ = task.await;

        // The job should have been triggered at least once
        assert!(handle.state().await == SchedulerState::Stopped);
    }

    #[tokio::test]
    async fn test_scheduler_respects_cross_job_dependencies() {
        let storage = InMemoryStorage::new();
        let mut scheduler = Scheduler::new(storage);

        // Upstream job
        let upstream = create_job("upstream", "Upstream Job");
        scheduler.register(upstream);

        // Downstream job depends on upstream
        let downstream = create_job("downstream", "Downstream Job")
            .with_dependency(JobDependency::new(JobId::new("upstream")));
        scheduler.register(downstream);

        let (handle, task) = scheduler.start().await;

        // Try to trigger downstream - should fail because upstream hasn't run
        let result = handle.trigger("downstream").await;
        assert!(matches!(
            result,
            Err(SchedulerError::DependencyNotSatisfied(_))
        ));

        // Trigger upstream
        let upstream_run = handle.trigger("upstream").await.unwrap();
        assert!(!upstream_run.as_uuid().is_nil());

        // Wait for upstream to complete
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Now downstream should work
        let downstream_run = handle.trigger("downstream").await.unwrap();
        assert!(!downstream_run.as_uuid().is_nil());

        handle.shutdown().await.unwrap();
        let _ = task.await;
    }

    #[tokio::test]
    async fn test_manual_job_trigger() {
        let storage = InMemoryStorage::new();
        let mut scheduler = Scheduler::new(storage);

        let job = create_job("manual", "Manual Job");
        scheduler.register(job);

        let (handle, task) = scheduler.start().await;

        // Trigger manually
        let run_id = handle.trigger("manual").await.unwrap();
        assert!(!run_id.as_uuid().is_nil());

        // Wait for completion
        tokio::time::sleep(Duration::from_millis(100)).await;

        handle.shutdown().await.unwrap();
        let _ = task.await;
    }

    #[tokio::test]
    async fn test_pause_and_resume_scheduler() {
        let storage = InMemoryStorage::new();
        let scheduler = Scheduler::new(storage).with_tick_interval(Duration::from_millis(50));

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

        handle.shutdown().await.unwrap();
        let _ = task.await;
    }

    #[tokio::test]
    async fn test_scheduler_recovers_interrupted_runs_on_startup() {
        let storage = InMemoryStorage::new();

        // Create some "interrupted" runs
        let run1 = StoredRun::new(RunId::new(), JobId::new("job1"));
        let run2 = StoredRun::new(RunId::new(), JobId::new("job2"));
        storage.save_run(run1.clone()).await.unwrap();
        storage.save_run(run2.clone()).await.unwrap();

        // Mark one as running (simulating interruption)
        let mut running = run1.clone();
        running.mark_running();
        storage.update_run(running).await.unwrap();

        let scheduler = Scheduler::new(storage);

        // Recover
        let recovered = scheduler.recover().await.unwrap();

        // Should have recovered 2 incomplete runs
        assert_eq!(recovered.len(), 2);
    }

    #[tokio::test]
    async fn test_scheduler_marks_interrupted_tasks_per_retry_policy() {
        let storage = Arc::new(InMemoryStorage::new());

        // Create an interrupted run
        let run_id = RunId::new();
        let mut run = StoredRun::new(run_id.clone(), JobId::new("job1"));
        run.mark_running();
        storage.save_run(run).await.unwrap();

        let scheduler = Scheduler::with_storage(Arc::clone(&storage));

        // Recover should mark runs as interrupted
        scheduler.recover().await.unwrap();

        // Check the run is now marked as interrupted
        let updated_run = storage.get_run(&run_id).await.unwrap();
        assert_eq!(updated_run.status, RunStatus::Interrupted);
    }

    #[tokio::test]
    async fn test_concurrent_job_runs_if_allowed() {
        let storage = InMemoryStorage::new();
        let mut scheduler = Scheduler::new(storage);

        // Job that allows 2 concurrent runs
        let job = create_job("concurrent", "Concurrent Job").with_max_concurrency(2);
        scheduler.register(job);

        let (handle, task) = scheduler.start().await;

        // Trigger twice - should both succeed
        let run1 = handle.trigger("concurrent").await.unwrap();
        let run2 = handle.trigger("concurrent").await.unwrap();

        assert!(!run1.as_uuid().is_nil());
        assert!(!run2.as_uuid().is_nil());
        assert_ne!(run1, run2);

        handle.shutdown().await.unwrap();
        let _ = task.await;
    }

    #[tokio::test]
    async fn test_max_concurrency_exceeded() {
        let storage = InMemoryStorage::new();
        let mut scheduler = Scheduler::new(storage);

        // Create a slow job that allows only 1 concurrent run
        struct SlowTask;

        #[async_trait]
        impl Task for SlowTask {
            fn name(&self) -> &str {
                "slow"
            }

            async fn execute(&self, _ctx: &mut TaskContext) -> Result<(), TaskError> {
                tokio::time::sleep(Duration::from_millis(500)).await;
                Ok(())
            }
        }

        let dag = DagBuilder::new("slow_dag", "Slow DAG")
            .add_task(Arc::new(SlowTask) as Arc<dyn Task>)
            .build()
            .unwrap();

        let job = Job::new("slow_job", "Slow Job", dag).with_max_concurrency(1);
        scheduler.register(job);

        let (handle, task) = scheduler.start().await;

        // First trigger should succeed
        let run1 = handle.trigger("slow_job").await.unwrap();
        assert!(!run1.as_uuid().is_nil());

        // Second trigger should fail (max concurrency = 1, and first is still running)
        let result = handle.trigger("slow_job").await;
        assert!(matches!(
            result,
            Err(SchedulerError::MaxConcurrentRunsExceeded(_))
        ));

        handle.shutdown().await.unwrap();
        let _ = task.await;
    }

    #[tokio::test]
    async fn test_scheduler_emits_events() {
        let event_bus = EventBus::new();
        let handler = RecordingHandler::new();
        event_bus.register(handler.clone()).await;

        let storage = InMemoryStorage::new();
        let mut scheduler = Scheduler::new(storage).with_event_bus(event_bus);

        let job = create_job("events", "Events Job");
        scheduler.register(job);

        let (handle, task) = scheduler.start().await;

        // Trigger a job
        handle.trigger("events").await.unwrap();

        // Wait for execution
        tokio::time::sleep(Duration::from_millis(100)).await;

        handle.shutdown().await.unwrap();
        let _ = task.await;

        // Check events
        let events = handler.events().await;
        assert!(!events.is_empty());

        // Should have JobStarted and JobCompleted events
        let has_started = events.iter().any(|e| matches!(e, Event::JobStarted { .. }));
        let has_completed = events
            .iter()
            .any(|e| matches!(e, Event::JobCompleted { .. }));

        assert!(has_started, "Should have JobStarted event");
        assert!(has_completed, "Should have JobCompleted event");
    }

    #[tokio::test]
    async fn test_trigger_nonexistent_job() {
        let storage = InMemoryStorage::new();
        let scheduler = Scheduler::new(storage);

        let (handle, task) = scheduler.start().await;

        let result = handle.trigger("nonexistent").await;
        assert!(matches!(result, Err(SchedulerError::JobNotFound(_))));

        handle.shutdown().await.unwrap();
        let _ = task.await;
    }

    #[tokio::test]
    async fn test_disabled_job_not_scheduled() {
        let storage = InMemoryStorage::new();
        let mut scheduler = Scheduler::new(storage).with_tick_interval(Duration::from_millis(50));

        // Disabled job with frequent schedule
        let job = create_scheduled_job("disabled", "Disabled Job", "@every 1s").with_enabled(false);
        scheduler.register(job);

        let (handle, task) = scheduler.start().await;

        // Wait a bit
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Manual trigger still works even when disabled
        // (this is a design choice - we could also block manual triggers)

        handle.shutdown().await.unwrap();
        let _ = task.await;
    }

    #[tokio::test]
    async fn test_scheduler_handle_clone() {
        let storage = InMemoryStorage::new();
        let mut scheduler = Scheduler::new(storage);

        let job = create_job("test", "Test Job");
        scheduler.register(job);

        let (handle, task) = scheduler.start().await;

        // Clone the handle
        let handle2 = handle.clone();

        // Both handles should work
        let run1 = handle.trigger("test").await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let run2 = handle2.trigger("test").await.unwrap();

        assert!(!run1.as_uuid().is_nil());
        assert!(!run2.as_uuid().is_nil());

        handle.shutdown().await.unwrap();
        let _ = task.await;
    }

    #[tokio::test]
    async fn test_global_max_concurrent_jobs() {
        let storage = InMemoryStorage::new();

        // Create slow tasks
        struct SlowTask;

        #[async_trait]
        impl Task for SlowTask {
            fn name(&self) -> &str {
                "slow"
            }

            async fn execute(&self, _ctx: &mut TaskContext) -> Result<(), TaskError> {
                tokio::time::sleep(Duration::from_millis(500)).await;
                Ok(())
            }
        }

        let dag1 = DagBuilder::new("dag1", "DAG 1")
            .add_task(Arc::new(SlowTask) as Arc<dyn Task>)
            .build()
            .unwrap();

        let dag2 = DagBuilder::new("dag2", "DAG 2")
            .add_task(Arc::new(SlowTask) as Arc<dyn Task>)
            .build()
            .unwrap();

        let mut scheduler = Scheduler::new(storage).with_max_concurrent_jobs(1);

        scheduler.register(Job::new("job1", "Job 1", dag1));
        scheduler.register(Job::new("job2", "Job 2", dag2));

        let (handle, task) = scheduler.start().await;

        // First job should succeed
        let run1 = handle.trigger("job1").await.unwrap();
        assert!(!run1.as_uuid().is_nil());

        // Second job should fail due to global limit
        let result = handle.trigger("job2").await;
        assert!(matches!(
            result,
            Err(SchedulerError::MaxConcurrentRunsExceeded(_))
        ));

        handle.shutdown().await.unwrap();
        let _ = task.await;
    }

    #[tokio::test]
    async fn test_job_with_within_window_dependency() {
        let storage = InMemoryStorage::new();
        let mut scheduler: Scheduler<InMemoryStorage> = Scheduler::new(storage);

        // Upstream job
        let upstream = create_job("upstream", "Upstream Job");
        scheduler.register(upstream);

        // Downstream requires upstream to have completed within 1 hour
        let downstream = create_job("downstream", "Downstream Job").with_dependency(
            JobDependency::with_condition(
                JobId::new("upstream"),
                DependencyCondition::WithinWindow(Duration::from_secs(3600)),
            ),
        );
        scheduler.register(downstream);

        let (handle, task) = scheduler.start().await;

        // Trigger upstream first
        handle.trigger("upstream").await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Now downstream should work (upstream completed within window)
        let result = handle.trigger("downstream").await;
        assert!(result.is_ok());

        handle.shutdown().await.unwrap();
        let _ = task.await;
    }
}
