# Petit

A minimal, lightweight task orchestrator with cron-like scheduling, written in Rust.

## Features

- **DAG-based execution** — Define task dependencies; independent tasks run in parallel
- **Cron scheduling** — Standard cron expressions with timezone support
- **YAML configuration** — Define jobs declaratively
- **Retry policies** — Configurable retries with fixed delays
- **Conditional execution** — Run tasks based on upstream success/failure
- **Cross-job dependencies** — Jobs can depend on other jobs
- **Event system** — Subscribe to lifecycle events (task started, completed, failed, etc.)
- **Pluggable storage** — In-memory (default) or SQLite for persistence
- **Concurrency control** — Limit parallel tasks and concurrent job runs

## Installation

```bash
cargo install --path .

# With SQLite support
cargo install --path . --features sqlite
```

## Quick Start

### 1. Create a job file

```yaml
# jobs/hello.yaml
id: hello_world
name: Hello World Job

tasks:
  - id: greet
    type: command
    command: echo
    args: ["Hello, World from Petit!"]
```

### 2. Run the scheduler

```bash
petit run jobs/
```

### 3. Or trigger a job manually

```bash
petit trigger jobs/ hello_world
```

## CLI Commands

```
petit run <jobs-dir>       # Run scheduler with jobs from directory
petit validate <jobs-dir>  # Validate job configurations
petit list <jobs-dir>      # List all jobs
petit trigger <jobs-dir> <job-id>  # Trigger a job manually
```

### Options for `run`

| Flag                  | Description                                     |
| --------------------- | ----------------------------------------------- |
| `-j, --max-jobs <N>`  | Maximum concurrent jobs (default: unlimited)    |
| `-t, --max-tasks <N>` | Maximum concurrent tasks per job (default: 4)   |
| `--tick-interval <N>` | Scheduler tick interval in seconds (default: 1) |

## Job Configuration

Jobs are defined in YAML files:

```yaml
id: data_pipeline
name: Data Pipeline
schedule: "0 0 2 * * *" # 2 AM daily (6-field cron: sec min hour day month weekday)
enabled: true
max_concurrency: 1

config:
  batch_size: 1000
  output_dir: /tmp/data

tasks:
  - id: extract
    type: command
    command: python
    args: ["scripts/extract.py"]
    environment:
      STAGE: extract

  - id: transform
    type: command
    command: python
    args: ["scripts/transform.py"]
    depends_on: [extract]
    condition: all_success
    retry:
      max_attempts: 3
      delay_secs: 10
      condition: always

  - id: notify
    type: command
    command: echo
    args: ["Pipeline done"]
    depends_on: [transform]
    condition: all_done # Runs regardless of upstream success/failure
```

### Task Conditions

| Condition     | Description                                           |
| ------------- | ----------------------------------------------------- |
| `always`      | Run if dependencies completed (default)               |
| `all_success` | Run only if all upstream tasks succeeded              |
| `on_failure`  | Run only if at least one upstream task failed         |
| `all_done`    | Run after dependencies complete, regardless of status |

### Retry Configuration

```yaml
retry:
  max_attempts: 3 # Number of retries (0 = no retries)
  delay_secs: 5 # Fixed delay between attempts
  condition: always # always | transient_only | never
```

### Cross-Job Dependencies

```yaml
id: downstream_job
name: Downstream Job
depends_on:
  - job_id: upstream_job
    condition: last_success # last_success | last_complete | within_window
```

## Library Usage

Use Petit as a library in your Rust application:

```rust
use petit::{
    Job, DagBuilder, CommandTask, Schedule,
    Scheduler, InMemoryStorage, EventBus, DagExecutor,
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Build a DAG
    let dag = DagBuilder::new("pipeline", "ETL Pipeline")
        .add_task(Arc::new(
            CommandTask::builder("echo")
                .name("extract")
                .args(["Extracting..."])
                .build()
        ))
        .add_task_with_deps(
            Arc::new(
                CommandTask::builder("echo")
                    .name("transform")
                    .args(["Transforming..."])
                    .build()
            ),
            &["extract"],
        )
        .build()?;

    // Create a job
    let job = Job::new("etl", "ETL Job", dag)
        .with_schedule(Schedule::new("0 */5 * * * *")?);  // Every 5 minutes

    // Set up scheduler
    let storage = InMemoryStorage::new();
    let mut scheduler = Scheduler::new(storage)
        .with_dag_executor(DagExecutor::with_concurrency(4));

    scheduler.register(job);

    // Start
    let (handle, task) = scheduler.start().await;

    // Trigger manually
    handle.trigger("etl").await?;

    // Shutdown
    handle.shutdown().await?;

    Ok(())
}
```

### CommandTask

`CommandTask` executes external commands with support for timeouts, environment variables, and retry policies:

```rust
use petit::{CommandTask, Environment, RetryPolicy};
use std::time::Duration;

// Simple command
let task = CommandTask::builder("echo")
    .arg("hello")
    .build();

// Command with all options
let task = CommandTask::builder("python")
    .name("process_data")
    .args(["-m", "etl.process", "--batch-size", "1000"])
    .env("DATABASE_URL", "postgres://localhost/db")
    .env("LOG_LEVEL", "info")
    .working_dir("/app")
    .timeout(Duration::from_secs(300))
    .retry_policy(RetryPolicy::fixed(3, Duration::from_secs(10)))
    .build();

// Using Environment object for multiple variables
let env = Environment::new()
    .with_var("AWS_REGION", "us-east-1")
    .with_var("S3_BUCKET", "my-bucket");

let task = CommandTask::builder("aws")
    .args(["s3", "sync", ".", "s3://my-bucket"])
    .environment(env)
    .timeout(Duration::from_secs(600))
    .build();
```

**Timeout Handling**: When a timeout occurs, the task returns a transient error that can trigger retries if configured.

**Output Capture**: stdout and stderr are captured and stored in the task context as `{task_name}.stdout` and `{task_name}.stderr`, accessible by downstream tasks.

### Custom Event Handler

```rust
use petit::{Event, EventHandler, EventBus};
use async_trait::async_trait;

struct MyHandler;

#[async_trait]
impl EventHandler for MyHandler {
    async fn handle(&self, event: &Event) {
        match event {
            Event::JobCompleted { job_id, success, duration, .. } => {
                println!("Job {} finished: success={}, took {:?}", job_id, success, duration);
            }
            Event::TaskFailed { task_id, error, .. } => {
                eprintln!("Task {} failed: {}", task_id, error);
            }
            _ => {}
        }
    }
}

// Register handler
let event_bus = EventBus::new();
event_bus.register(Arc::new(MyHandler)).await;
```

### SQLite Storage

Enable persistent storage with the `sqlite` feature:

```rust
use petit::SqliteStorage;

let storage = SqliteStorage::new("petit.db").await?;
// or in-memory for testing:
let storage = SqliteStorage::in_memory().await?;

let scheduler = Scheduler::new(storage);
```

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                        CLI                              │
│              run | validate | list | trigger            │
└─────────────────────────┬───────────────────────────────┘
                          │
┌─────────────────────────▼───────────────────────────────┐
│                     Scheduler                           │
│         • Cron scheduling    • Job dependencies         │
│         • Pause/resume       • Concurrency control      │
└─────────────────────────┬───────────────────────────────┘
                          │
┌─────────────────────────▼───────────────────────────────┐
│                    DagExecutor                          │
│         • Topological ordering  • Parallel execution    │
│         • Task conditions       • Context propagation   │
└─────────────────────────┬───────────────────────────────┘
                          │
┌─────────────────────────▼───────────────────────────────┐
│                    TaskExecutor                         │
│         • Retry logic          • Timeout handling       │
│         • CommandTask          • Custom Task trait      │
└─────────────────────────────────────────────────────────┘
                          │
          ┌───────────────┼───────────────┐
          ▼               ▼               ▼
     ┌─────────┐    ┌──────────┐    ┌───────────┐
     │ Storage │    │ EventBus │    │  Context  │
     └─────────┘    └──────────┘    └───────────┘
```

## Development

```bash
# Run tests
cargo test

# Run tests with SQLite
cargo test --features sqlite

# Build release
cargo build --release

# Run with debug logging
RUST_LOG=debug cargo run -- run examples/jobs/
```

## License

MIT
