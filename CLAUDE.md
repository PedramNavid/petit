# Petit - Task Orchestrator

A minimal, lightweight task orchestrator with cron-like scheduling written in Rust.

## Project Structure

```
src/
├── api/         # HTTP REST API (axum-based)
│   ├── mod.rs       # Router and server setup
│   ├── handlers.rs  # Request handlers
│   ├── responses.rs # Response types (JSON serialization)
│   └── errors.rs    # API error types
├── config/      # YAML configuration loading and job builders
├── core/        # Core types: Job, Task, DAG, Schedule, Retry policies
├── events/      # Event bus for task/job lifecycle events
├── execution/   # Task executors and DAG execution engine
├── scheduler/   # Cron-based scheduler engine
├── storage/     # Storage backends (in-memory, SQLite)
├── testing/     # Test harness and mock utilities
├── tui/         # Terminal UI dashboard (ratatui-based)
│   ├── app.rs       # Application state and logic
│   ├── event.rs     # Event loop (keyboard, tick)
│   ├── reader.rs    # Read-only SQLite interface
│   ├── theme.rs     # Color theme definitions
│   └── ui/          # UI components (tabs, widgets)
├── lib.rs       # Public API exports
└── main.rs      # CLI entry point (pt binary)

tests/
├── integration.rs        # Integration test runner
└── integration/
    ├── api.rs            # HTTP API integration tests
    ├── recovery.rs       # Recovery scenario tests
    ├── resources.rs      # Resource contention tests
    └── workflow.rs       # End-to-end workflow tests
```

## Development Commands

Use the Makefile for all development tasks:

```bash
make fmt        # Format code with rustfmt
make fmt-check  # Check formatting without modifying
make lint       # Run clippy with warnings as errors
make test       # Run all tests
make check      # Quick compile check
make build      # Build release binary
make ci         # Run all CI checks (fmt-check, lint, test)
make all        # Same as ci
```

## Running the Application

### CLI (pt binary)

```bash
# Run scheduler with jobs from a directory (API server starts by default)
cargo run -- run ./jobs

# Run scheduler without API server (requires api feature, enabled by default)
cargo run -- run ./jobs --no-api

# Run scheduler with custom API settings
cargo run -- run ./jobs --api-port 8080 --api-host 0.0.0.0

# Trigger a specific job once and exit
cargo run -- trigger ./jobs my_job
```

**Note:** The `--no-api`, `--api-port`, and `--api-host` flags require the `api` feature to be enabled (which is the default). If compiling with `--no-default-features`, these flags will not be available.

### TUI Dashboard

The TUI provides a read-only dashboard for monitoring the scheduler:

```bash
# Build and run the TUI (requires sqlite feature)
cargo run --bin petit-tui --features tui -- --db ./petit.db
```

### Running Scheduler + TUI Together

The scheduler and TUI are separate processes that share a SQLite database:

```bash
# Terminal 1: Start the scheduler with SQLite storage
cargo run -- run --jobs ./jobs --db ./petit.db --api

# Terminal 2: Start the TUI dashboard (read-only)
cargo run --bin petit-tui --features tui -- --db ./petit.db
```

The TUI connects to the database in read-only mode and polls for updates.

## Code Style

- Run `make fmt` before committing
- All clippy warnings are treated as errors (`-D warnings`)
- Use `--all-features` when running checks to include all features

## CI/CD

GitHub Actions runs on every push and PR to main:
- **Format check**: Verifies code is formatted with rustfmt
- **Lint**: Runs clippy with all features enabled
- **Test**: Runs the full test suite

## Features

- `api` - Enables HTTP REST API and related CLI flags (`--no-api`, `--api-port`, `--api-host`). **Enabled by default.**
- `sqlite` - Enables SQLite storage backend and `--db` CLI flag
- `tui` - Enables terminal UI dashboard (includes sqlite)

### Building Without Default Features

```bash
# Build without API support (no API server or API-related CLI flags)
cargo build --no-default-features

# Build with only SQLite support (no API)
cargo build --no-default-features --features sqlite
```

## Testing

Tests are co-located with source files. The `testing` module provides:
- `TestHarness` for integration tests
- `MockTaskContext` for unit tests
- `ExecutionTracker` for verifying task execution order

Integration tests in `tests/integration/` cover:
- API endpoints (`api.rs`)
- Workflow execution (`workflow.rs`)
- Recovery scenarios (`recovery.rs`)
- Resource contention (`resources.rs`)

## API Endpoints

When the `api` feature is enabled (default), an HTTP REST API server starts automatically unless `--no-api` is specified. The following endpoints are available:

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/health` | GET | Health check |
| `/api/scheduler/state` | GET | Get scheduler state |
| `/api/scheduler/pause` | POST | Pause scheduler |
| `/api/scheduler/resume` | POST | Resume scheduler |
| `/api/jobs` | GET | List all jobs |
| `/api/jobs/{id}` | GET | Get job details |
| `/api/jobs/{id}/trigger` | POST | Trigger a job |
| `/api/jobs/{id}/runs` | GET | List runs for a job |
| `/api/runs/{id}` | GET | Get run details |
| `/api/runs/{id}/tasks` | GET | List task states for a run |
