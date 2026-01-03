# Petit - Task Orchestrator

A minimal, lightweight task orchestrator with cron-like scheduling written in Rust.

## Project Structure

```
src/
├── config/      # YAML configuration loading and job builders
├── core/        # Core types: Job, Task, DAG, Schedule, Retry policies
├── events/      # Event bus for task/job lifecycle events
├── execution/   # Task executors and DAG execution engine
├── scheduler/   # Cron-based scheduler engine
├── storage/     # Storage backends (in-memory, SQLite)
├── testing/     # Test harness and mock utilities
├── lib.rs       # Public API exports
└── main.rs      # CLI entry point
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

## Code Style

- Run `make fmt` before committing
- All clippy warnings are treated as errors (`-D warnings`)
- Use `--all-features` when running checks to include SQLite feature

## CI/CD

GitHub Actions runs on every push and PR to main:
- **Format check**: Verifies code is formatted with rustfmt
- **Lint**: Runs clippy with all features enabled
- **Test**: Runs the full test suite

## Features

- `sqlite` - Enables SQLite storage backend (optional)

## Testing

Tests are co-located with source files. The `testing` module provides:
- `TestHarness` for integration tests
- `MockTaskContext` for unit tests
- `ExecutionTracker` for verifying task execution order
