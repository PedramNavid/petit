# Petit - Project Context

## Overview

Petit is a minimal, lightweight task orchestrator written in Rust. It provides cron-like scheduling with a text-based user interface, designed for single-machine deployments.

## Key Design Decisions

- **Async execution** with configurable concurrency limits
- **Resource management** via abstract slots (e.g., "gpu", "db_conn") and system constraints (CPU, memory)
- **Fixed delay retry** policy for failed tasks
- **YAML configuration** for job definitions
- **Extended cron** syntax (6-field with seconds, timezone support)
- **Key-value context** for inter-task data flow
- **Cross-job dependencies** supported
- **TUI** available via feature flag (`--features tui`)

## Architecture

```
src/
├── core/           # Types, Task trait, DAG, scheduling
├── execution/      # Executor, resource management, command tasks
├── events/         # Lifecycle events
├── storage/        # In-memory and SQLite backends
├── config/         # YAML parsing
├── scheduler/      # Main scheduling loop
└── testing/        # Test harness utilities
```

## Development Approach

**Test-driven development**: Tests are written first as executable documentation defining the API contract. Implementation follows to make tests pass.

**Git workflow**:
- Always work on feature branches, never commit directly to `main`
- Branch naming: `phase-N-description` (e.g., `phase-1-core-types`)
- Create a PR for every feature/phase
- Merge to `main` only after review

See `PLAN.md` for the phased implementation plan.
See `SPEC.md` for detailed technical specification.

## Current Status

**Phase 1 Complete**: Core types and Task abstraction (test stubs only)
- `TaskId`, `JobId`, `RunId`, `DagId`
- `ResourceRequirements`, `SystemResources`
- `RetryPolicy`, `RetryCondition`
- `TaskContext`, `ContextReader`, `ContextWriter`
- `Task` trait, `TaskError`

## Running Tests

```bash
cargo test
```

Note: Tests will panic with `todo!()` until implementations are complete.

## Building

```bash
cargo build
cargo build --features tui  # With TUI support (not yet implemented)
```
