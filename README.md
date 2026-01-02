# Petit

A minimal, lightweight task orchestrator written in Rust. Petit provides cron-like scheduling with DAG-based task dependencies, designed for single-machine deployments.

## Features

- **DAG-based workflows** - Define task dependencies with directed acyclic graphs
- **Flexible scheduling** - Extended cron syntax with seconds precision, shortcuts (`@daily`, `@hourly`), and intervals (`@every 5m`)
- **Timezone support** - Schedule jobs in any timezone
- **Cross-job dependencies** - Jobs can depend on successful completion of other jobs
- **Retry policies** - Configurable retry with fixed delays
- **Resource management** - Concurrency limits at task, job, and global levels
- **YAML configuration** - Define jobs declaratively
- **Pluggable storage** - In-memory (default) or SQLite backends
- **Event system** - Lifecycle events for observability
- **Recovery** - Automatic recovery of interrupted runs on restart

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
petit = "0.1"

# Optional: Enable SQLite storage
petit = { version = "0.1", features = ["sqlite"] }
```

## Quick Start

### Define a Task

```rust
use petit::{Task, TaskContext, TaskError};
use async_trait::async_trait;

struct ExtractTask;

#[async_trait]
impl Task for ExtractTask {
    fn name(&self) -> &str {
        "extract"
    }

    async fn execute(&self, ctx: &mut TaskContext) -> Result<(), TaskError> {
        // Your task logic here
        ctx.outputs.set("records_count", 1000)?;
        Ok(())
    }
}
```

### Build a DAG

```rust
use petit::{DagBuilder, TaskCondition};
use std::sync::Arc;

let dag = DagBuilder::new("etl_pipeline", "Daily ETL")
    .add_task(Arc::new(ExtractTask))
    .add_task_with_deps(Arc::new(TransformTask), &["extract"])
    .add_task_with_deps(Arc::new(LoadTask), &["transform"])
    .build()?;
```

### Create and Schedule a Job

```rust
use petit::{Job, Schedule, Scheduler, InMemoryStorage};

// Create a job with a schedule
let schedule = Schedule::new("0 2 * * *")?;  // 2 AM daily
let job = Job::new("daily_etl", "Daily ETL Job", dag)
    .with_schedule(schedule);

// Set up the scheduler
let storage = InMemoryStorage::new();
let mut scheduler = Scheduler::new(storage);
scheduler.register(job);

// Start the scheduler
let (handle, task) = scheduler.start().await;

// Trigger manually if needed
handle.trigger("daily_etl").await?;

// Pause/resume
handle.pause().await?;
handle.resume().await?;

// Shutdown gracefully
handle.shutdown().await?;
```

### Use External Commands

```rust
use petit::CommandTaskBuilder;

let task = CommandTaskBuilder::new("backup", "pg_dump")
    .args(&["-h", "localhost", "-d", "mydb", "-f", "backup.sql"])
    .working_dir("/var/backups")
    .timeout(Duration::from_secs(3600))
    .retry(3, Duration::from_secs(60))
    .build();
```

## YAML Configuration

Define jobs in YAML:

```yaml
id: daily_etl
name: Daily ETL Pipeline
schedule: "0 2 * * *"

tasks:
  - id: extract
    name: Extract Data
    type: command
    command: python
    args:
      - scripts/extract.py
    retry:
      max_attempts: 3
      delay_secs: 60

  - id: transform
    name: Transform Data
    type: command
    command: python
    args:
      - scripts/transform.py
    depends_on:
      - extract

  - id: load
    name: Load Data
    type: command
    command: python
    args:
      - scripts/load.py
    depends_on:
      - transform
    condition: all_success
```

Load and use:

```rust
use petit::YamlLoader;

let config = YamlLoader::parse_job_config(yaml_str)?;
```

## Schedule Expressions

Petit supports multiple schedule formats:

| Format | Example | Description |
|--------|---------|-------------|
| 5-field cron | `0 2 * * *` | Standard cron (minute hour day month weekday) |
| 6-field cron | `30 0 2 * * *` | Extended cron with seconds |
| Shortcuts | `@daily`, `@hourly`, `@weekly` | Common intervals |
| Intervals | `@every 5m`, `@every 1h30m` | Fixed intervals |

Timezone support:

```rust
let schedule = Schedule::with_timezone("0 9 * * *", "America/New_York")?;
```

## Task Conditions

Control when tasks run based on upstream results:

```rust
use petit::TaskCondition;

// Run only if all upstream tasks succeeded (default)
.add_task_with_deps_and_condition(task, &["upstream"], TaskCondition::AllSuccess)

// Run only if any upstream task failed
.add_task_with_deps_and_condition(cleanup, &["main"], TaskCondition::OnFailure)

// Run regardless of upstream status
.add_task_with_deps_and_condition(notify, &["main"], TaskCondition::AllDone)
```

## Cross-Job Dependencies

Jobs can depend on other jobs:

```rust
use petit::{Job, JobDependency, DependencyCondition};

let downstream = Job::new("report", "Daily Report", dag)
    .with_dependency(JobDependency::new(JobId::new("daily_etl")))
    .with_dependency(JobDependency::with_condition(
        JobId::new("data_validation"),
        DependencyCondition::WithinWindow(Duration::from_secs(3600)),
    ));
```

## Concurrency Control

Limit concurrent execution:

```rust
// Per-job limit
let job = Job::new("heavy_job", "Heavy Job", dag)
    .with_max_concurrency(1);  // Only one instance at a time

// Global scheduler limit
let scheduler = Scheduler::new(storage)
    .with_max_concurrent_jobs(5);

// DAG executor concurrency
let executor = DagExecutor::with_concurrency(4);
```

## Storage Backends

### In-Memory (Default)

```rust
use petit::InMemoryStorage;

let storage = InMemoryStorage::new();
```

### SQLite

```rust
use petit::SqliteStorage;

let storage = SqliteStorage::new("petit.db").await?;
// Or in-memory SQLite:
let storage = SqliteStorage::in_memory().await?;
```

## Event Handling

Subscribe to lifecycle events:

```rust
use petit::{Event, EventBus, EventHandler};
use async_trait::async_trait;

struct LoggingHandler;

#[async_trait]
impl EventHandler for LoggingHandler {
    async fn handle(&self, event: &Event) {
        match event {
            Event::JobStarted { job_id, run_id, .. } => {
                println!("Job {} started (run: {})", job_id, run_id);
            }
            Event::JobCompleted { job_id, success, duration, .. } => {
                println!("Job {} completed: success={}, duration={:?}",
                    job_id, success, duration);
            }
            _ => {}
        }
    }
}

let event_bus = EventBus::new();
event_bus.register(Arc::new(LoggingHandler)).await;

let scheduler = Scheduler::new(storage)
    .with_event_bus(event_bus);
```

## Recovery

Petit automatically recovers from interruptions:

```rust
let scheduler = Scheduler::new(storage);

// Recover interrupted runs on startup
let recovered = scheduler.recover().await?;
println!("Recovered {} interrupted runs", recovered.len());

// Then start normal operation
let (handle, task) = scheduler.start().await;
```

## Project Status

Petit is under active development. Current status:

- [x] Core types and Task abstraction
- [x] DAG structure and validation
- [x] Command task execution
- [x] Resource management
- [x] Task executor with retries
- [x] DAG executor
- [x] Event system
- [x] In-memory storage
- [x] SQLite storage
- [x] Schedule parsing
- [x] Job definitions
- [x] YAML configuration
- [x] Scheduler engine
- [ ] Test harness utilities
- [ ] TUI (planned)

## License

MIT

## Contributing

Contributions are welcome! Please see the [PLAN.md](PLAN.md) for the development roadmap.
