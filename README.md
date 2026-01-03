# Petit

A minimal, lightweight task orchestrator with cron-like scheduling, written in Rust.

## What .. and why?

I wanted something to orchestrate simple tasks on embedded hardware (Turing PI cluster with RK1 and Raspberry PIs). Low-memory and efficiency were important, but I also wanted something that was usable and friendly.

Importantly, I needed

- Tasks and Jobs that can form dependencies
- Simple observability
- Concurrency
- YAML configuration

And so I built this.

## Should I use this?

No! I built this to see what it would take to build an orchestrator with Claude Code using Opus 4.5. It works for me but please do not use it for you.

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

# With TUI support
cargo install --path . --features tui,sqlite
```

## Quick Start

There are example jobs in ./examples. You can use those or follow these instructions:

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
pt run jobs/
```

### 3. Or trigger a job manually

```bash
pt trigger jobs/ hello_world
```

### 4. Run with a database

By default, this runs in-memory with no persistence!

```bash
pt trigger jobs/ hello_world --db petit.db
```

## CLI Commands

```
pt run <jobs-dir>       # Run scheduler with jobs from directory
pt validate <jobs-dir>  # Validate job configurations
pt list <jobs-dir>      # List all jobs
pt trigger <jobs-dir> <job-id>  # Trigger a job manually
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

```

```
