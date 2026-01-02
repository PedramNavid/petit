# Petit - Minimal Task Orchestrator Specification

## Overview

Petit is a lightweight, memory-efficient task orchestrator written in Rust. It provides cron-like scheduling capabilities with a text-based user interface, designed for single-machine deployments with extensible storage backends.

## Core Concepts

### Task

A Task is the fundamental unit of work. Each task has:

- **Compute Function**: Either a Rust trait implementation or an external command
- **Environment**: Key-value environment variables passed during execution (similar to Docker/Kubernetes)
- **Retry Policy**: Fixed delay retry with configurable attempts and interval
- **Metadata**: Name, description, tags for organization

```rust
#[async_trait]
pub trait Task: Send + Sync {
    /// Unique identifier for this task type
    fn name(&self) -> &str;

    /// Execute the task with the given context
    async fn execute(&self, ctx: &mut TaskContext) -> Result<(), TaskError>;

    /// Environment variables for this task
    fn environment(&self) -> Environment {
        Environment::default()
    }

    /// Retry policy for this task
    fn retry_policy(&self) -> RetryPolicy {
        RetryPolicy::default()
    }
}
```

### External Command Task

For wrapping shell commands and external executables:

```rust
pub struct CommandTask {
    name: String,
    program: String,
    args: Vec<String>,
    environment: Environment,
    working_dir: Option<PathBuf>,
    retry_policy: RetryPolicy,
}
```

### Environment

Tasks receive environment variables for configuration, credentials, and runtime values:

```rust
pub struct Environment {
    /// Environment variables as key-value pairs
    vars: HashMap<String, String>,
}

impl Environment {
    /// Create an empty environment
    pub fn new() -> Self;

    /// Builder: add an environment variable
    pub fn with_var(self, key: &str, value: &str) -> Self;

    /// Merge another environment (other values override)
    pub fn merged_with(&self, other: &Environment) -> Self;
}
```

Environment variables cascade: job-level environment is merged with task-level environment, with task values taking precedence.

### Retry Policy

Fixed delay retry strategy:

```rust
pub struct RetryPolicy {
    /// Maximum number of retry attempts (0 = no retries)
    pub max_attempts: u32,

    /// Fixed delay between retry attempts
    pub delay: Duration,

    /// Whether to retry on specific error types only
    pub retry_on: RetryCondition,
}

pub enum RetryCondition {
    /// Retry on any error
    Always,
    /// Retry only on transient errors (timeouts, resource unavailable)
    TransientOnly,
    /// Never retry
    Never,
}
```

### DAG (Directed Acyclic Graph)

A DAG defines task dependencies and execution order:

```rust
pub struct Dag {
    /// Unique identifier
    pub id: DagId,

    /// Human-readable name
    pub name: String,

    /// Task nodes in the graph
    pub tasks: HashMap<TaskId, TaskNode>,

    /// Edges representing dependencies (task -> depends_on)
    pub edges: HashMap<TaskId, Vec<TaskId>>,
}

pub struct TaskNode {
    pub id: TaskId,
    pub task: Arc<dyn Task>,
    /// Optional condition for execution
    pub condition: Option<TaskCondition>,
}

pub enum TaskCondition {
    /// Always run
    Always,
    /// Run only if all upstream tasks succeeded
    AllSuccess,
    /// Run only if at least one upstream task failed
    OnFailure,
    /// Run regardless of upstream status
    AllDone,
}
```

### Job

A Job is a scheduled DAG execution:

```rust
pub struct Job {
    /// Unique identifier
    pub id: JobId,

    /// Human-readable name
    pub name: String,

    /// The DAG to execute
    pub dag: Dag,

    /// Cron schedule (extended syntax)
    pub schedule: Option<Schedule>,

    /// Dependencies on other jobs
    pub depends_on: Vec<JobDependency>,

    /// Global concurrency limit for this job's tasks
    pub max_concurrency: Option<usize>,

    /// Job-level configuration passed to all tasks
    pub config: HashMap<String, Value>,
}

pub struct JobDependency {
    pub job_id: JobId,
    /// Wait for specific execution or just last successful
    pub condition: DependencyCondition,
}

pub enum DependencyCondition {
    /// Wait for the last execution to complete successfully
    LastSuccess,
    /// Wait for any execution within a time window
    WithinWindow(Duration),
}
```

### Schedule (Extended Cron)

Extended cron syntax with seconds precision:

```rust
pub struct Schedule {
    /// Cron expression (6 fields: second minute hour day month weekday)
    pub expression: String,

    /// Timezone for schedule interpretation
    pub timezone: chrono_tz::Tz,
}

// Supported shortcuts:
// @yearly, @annually -> 0 0 0 1 1 *
// @monthly -> 0 0 0 1 * *
// @weekly -> 0 0 0 * * 0
// @daily, @midnight -> 0 0 0 * * *
// @hourly -> 0 0 * * * *
// @every <duration> -> custom interval
```

## Execution Model

### Executor

The executor manages task execution with concurrency control:

```rust
pub struct Executor {
    /// Global concurrency limit
    max_concurrency: usize,

    /// Task execution runtime
    runtime: tokio::runtime::Handle,
}

impl Executor {
    /// Execute a DAG, respecting dependencies and concurrency limits
    pub async fn execute_dag(&self, dag: &Dag, ctx: ExecutionContext) -> DagResult;

    /// Execute a single task with environment variables
    pub async fn execute_task(&self, task: &dyn Task, ctx: TaskContext, env: Environment) -> TaskResult;
}
```

### Concurrency Control

- Global executor concurrency limit
- Per-job concurrency limit
- Semaphore-based task scheduling

### Task Context

Shared key-value store for inter-task communication:

```rust
pub struct TaskContext {
    /// Read from upstream task outputs
    pub inputs: ContextReader,

    /// Write outputs for downstream tasks
    pub outputs: ContextWriter,

    /// Job-level configuration
    pub config: Arc<HashMap<String, Value>>,

    /// Execution metadata
    pub execution: ExecutionMetadata,
}

pub struct ContextReader {
    store: Arc<RwLock<HashMap<String, Value>>>,
}

impl ContextReader {
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Result<T, ContextError>;
    pub fn get_optional<T: DeserializeOwned>(&self, key: &str) -> Option<T>;
}

pub struct ContextWriter {
    store: Arc<RwLock<HashMap<String, Value>>>,
    task_id: TaskId,
}

impl ContextWriter {
    /// Write a value, prefixed with task_id automatically
    pub fn set<T: Serialize>(&self, key: &str, value: T) -> Result<(), ContextError>;
}
```

## Event Model

Lifecycle events for observability:

```rust
pub enum Event {
    // Task events
    TaskStarted { job_id: JobId, task_id: TaskId, run_id: RunId, timestamp: DateTime<Utc> },
    TaskCompleted { job_id: JobId, task_id: TaskId, run_id: RunId, timestamp: DateTime<Utc>, duration: Duration },
    TaskFailed { job_id: JobId, task_id: TaskId, run_id: RunId, timestamp: DateTime<Utc>, error: String },
    TaskRetrying { job_id: JobId, task_id: TaskId, run_id: RunId, attempt: u32, next_retry: DateTime<Utc> },

    // Job events
    JobStarted { job_id: JobId, run_id: RunId, timestamp: DateTime<Utc> },
    JobCompleted { job_id: JobId, run_id: RunId, timestamp: DateTime<Utc>, duration: Duration },
    JobFailed { job_id: JobId, run_id: RunId, timestamp: DateTime<Utc>, failed_tasks: Vec<TaskId> },
}

pub trait EventHandler: Send + Sync {
    fn handle(&self, event: &Event);
}

pub struct EventBus {
    handlers: Vec<Arc<dyn EventHandler>>,
}
```

## Storage Abstraction

Pluggable storage backends:

```rust
#[async_trait]
pub trait Storage: Send + Sync {
    // Job management
    async fn save_job(&self, job: &Job) -> Result<(), StorageError>;
    async fn get_job(&self, id: &JobId) -> Result<Option<Job>, StorageError>;
    async fn list_jobs(&self) -> Result<Vec<Job>, StorageError>;
    async fn delete_job(&self, id: &JobId) -> Result<(), StorageError>;

    // Run history
    async fn save_run(&self, run: &JobRun) -> Result<(), StorageError>;
    async fn get_run(&self, id: &RunId) -> Result<Option<JobRun>, StorageError>;
    async fn list_runs(&self, job_id: &JobId, limit: usize) -> Result<Vec<JobRun>, StorageError>;

    // Task state
    async fn save_task_state(&self, state: &TaskState) -> Result<(), StorageError>;
    async fn get_task_state(&self, run_id: &RunId, task_id: &TaskId) -> Result<Option<TaskState>, StorageError>;

    // Incomplete run recovery
    async fn get_incomplete_runs(&self) -> Result<Vec<JobRun>, StorageError>;
    async fn mark_run_interrupted(&self, run_id: &RunId) -> Result<(), StorageError>;
}

pub struct JobRun {
    pub id: RunId,
    pub job_id: JobId,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub task_states: HashMap<TaskId, TaskState>,
}

pub enum RunStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Interrupted,
}

pub struct TaskState {
    pub task_id: TaskId,
    pub status: TaskStatus,
    pub attempts: u32,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub outputs: HashMap<String, Value>,
}
```

### In-Memory Storage

```rust
pub struct InMemoryStorage {
    jobs: RwLock<HashMap<JobId, Job>>,
    runs: RwLock<HashMap<RunId, JobRun>>,
}
```

### SQLite Storage

```rust
pub struct SqliteStorage {
    pool: SqlitePool,
}
```

## Configuration (YAML)

### Job Definition

```yaml
# jobs/daily_etl.yaml
name: daily_etl
schedule: "0 0 2 * * *"  # 2 AM daily
timezone: America/Los_Angeles
max_concurrency: 4

depends_on:
  - job: data_ingestion
    condition: last_success

# Job-level environment (inherited by all tasks)
environment:
  LOG_LEVEL: info
  DATA_DIR: /var/data

config:
  data_path: /var/data
  output_format: parquet

tasks:
  extract:
    type: command
    program: python
    args: ["scripts/extract.py"]
    environment:
      INPUT_PATH: "${config.data_path}/raw"
      DATABASE_URL: postgres://localhost/source
    retry:
      max_attempts: 3
      delay_seconds: 60

  transform:
    type: command
    program: python
    args: ["scripts/transform.py"]
    depends_on: [extract]
    environment:
      SPARK_MASTER: local[4]

  load:
    type: command
    program: python
    args: ["scripts/load.py"]
    depends_on: [transform]
    condition: all_success
    environment:
      OUTPUT_FORMAT: parquet
```

### Global Configuration

```yaml
# petit.yaml
executor:
  max_concurrency: 8

# Global environment inherited by all jobs
environment:
  TZ: UTC

storage:
  type: sqlite
  path: /var/lib/petit/petit.db

scheduler:
  tick_interval_ms: 1000

logging:
  level: info
  format: json
```

## Recovery Behavior

On orchestrator restart:

1. Load incomplete runs from storage
2. Mark all `Running` task states as `Interrupted`
3. For each incomplete run:
   - If retry policy allows, re-queue interrupted tasks
   - Otherwise, mark task as `Failed`
4. Resume scheduling for pending runs

```rust
impl Orchestrator {
    pub async fn recover(&self) -> Result<(), RecoveryError> {
        let incomplete = self.storage.get_incomplete_runs().await?;

        for run in incomplete {
            for (task_id, state) in &run.task_states {
                if state.status == TaskStatus::Running {
                    self.storage.mark_task_interrupted(&run.id, task_id).await?;

                    // Check retry policy
                    let task = self.get_task(&run.job_id, task_id)?;
                    if state.attempts < task.retry_policy().max_attempts {
                        self.scheduler.enqueue_retry(&run.id, task_id).await?;
                    } else {
                        self.storage.mark_task_failed(&run.id, task_id, "Interrupted").await?;
                    }
                }
            }
        }
        Ok(())
    }
}
```

## TUI (Feature Flag)

Enabled via `--features tui` Cargo flag.

### Views

1. **Dashboard**: Overview of all jobs, next scheduled runs, recent failures
2. **Job List**: All jobs with status, last run, next run
3. **Job Detail**: DAG visualization, task states, run history
4. **Run Detail**: Task execution timeline, logs, outputs
5. **Logs**: Filterable log viewer

### Controls

- Trigger manual job execution
- Cancel running jobs
- Pause/resume scheduler
- View task outputs and errors

## Module Structure

```
src/
├── lib.rs              # Public API
├── core/
│   ├── mod.rs
│   ├── task.rs         # Task trait
│   ├── context.rs      # TaskContext, key-value store
│   ├── environment.rs  # Environment variables
│   ├── dag.rs          # DAG structure, validation
│   ├── job.rs          # Job definition
│   └── schedule.rs     # Cron parsing, scheduling
├── execution/
│   ├── mod.rs
│   └── executor.rs     # Task execution engine
├── storage/
│   ├── mod.rs
│   ├── traits.rs       # Storage trait
│   ├── memory.rs       # In-memory implementation
│   └── sqlite.rs       # SQLite implementation
├── events/
│   ├── mod.rs
│   └── bus.rs          # Event types, EventBus
├── config/
│   ├── mod.rs
│   └── yaml.rs         # YAML parsing
├── scheduler/
│   ├── mod.rs
│   └── engine.rs       # Main scheduling loop
└── tui/                # Feature-gated
    ├── mod.rs
    ├── app.rs
    └── views/
```

## Testing Strategy

### Unit Tests (Mock Context)

```rust
#[cfg(test)]
mod tests {
    use petit::testing::*;

    #[tokio::test]
    async fn test_task_execution() {
        let mut ctx = MockTaskContext::new();
        ctx.set_input("upstream_value", 42);

        let task = MyTask::new();
        let result = task.execute(&mut ctx).await;

        assert!(result.is_ok());
        assert_eq!(ctx.get_output::<i32>("result"), Some(84));
    }
}
```

### Integration Tests (In-Memory Harness)

```rust
#[cfg(test)]
mod integration {
    use petit::testing::TestHarness;

    #[tokio::test]
    async fn test_dag_execution() {
        let harness = TestHarness::new()
            .with_deterministic_time()
            .with_in_memory_storage();

        let dag = harness.load_dag("fixtures/simple_dag.yaml");
        let result = harness.execute_dag(&dag).await;

        assert!(result.is_success());
        assert_eq!(result.completed_tasks(), 3);

        // Verify execution order
        let timeline = result.execution_timeline();
        assert!(timeline.completed_before("extract", "transform"));
        assert!(timeline.completed_before("transform", "load"));
    }

    #[tokio::test]
    async fn test_retry_on_failure() {
        let harness = TestHarness::new()
            .with_deterministic_time();

        let task = FailingTask::new(failures: 2); // Fails twice, then succeeds
        let dag = DagBuilder::new()
            .add_task(task.with_retry(max_attempts: 3, delay: Duration::from_secs(1)))
            .build();

        let result = harness.execute_dag(&dag).await;

        assert!(result.is_success());
        assert_eq!(result.task_attempts("failing_task"), 3);
    }
}
```

## Error Types

```rust
#[derive(Debug, thiserror::Error)]
pub enum PetitError {
    #[error("Task error: {0}")]
    Task(#[from] TaskError),

    #[error("DAG validation error: {0}")]
    DagValidation(String),

    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    #[error("Schedule parse error: {0}")]
    Schedule(String),
}

#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("Execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Timeout after {0:?}")]
    Timeout(Duration),

    #[error("Command exited with code {0}")]
    CommandFailed(i32),

    #[error("Context error: {0}")]
    Context(#[from] ContextError),
}
```

## Dependencies

```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
serde_yaml = "0.9"
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
chrono-tz = "0.8"
cron = "0.12"
thiserror = "1"
tracing = "0.1"
uuid = { version = "1", features = ["v4", "serde"] }

[dependencies.sqlx]
version = "0.7"
features = ["runtime-tokio", "sqlite"]
optional = true

[dependencies.ratatui]
version = "0.26"
optional = true

[features]
default = ["sqlite"]
sqlite = ["sqlx"]
tui = ["ratatui"]
```
