# Petit Makefile
# Common targets for running the orchestrator and TUI

# Configuration
JOBS_DIR ?= examples/jobs
DB_FILE ?= petit.db

.PHONY: all check fmt fmt-check lint test build build-tui run run-sqlite tui clean ci help

help:
	@echo "Petit - Task Orchestrator"
	@echo ""
	@echo "Usage:"
	@echo "  make build        Build the petit binary (release)"
	@echo "  make build-tui    Build with TUI support (includes sqlite)"
	@echo "  make run          Run scheduler with in-memory storage"
	@echo "  make run-sqlite   Run scheduler with SQLite storage (DB_FILE=$(DB_FILE))"
	@echo "  make tui          Run the TUI dashboard (requires run-sqlite in another terminal)"
	@echo "  make test         Run all tests"
	@echo "  make lint         Run clippy linter"
	@echo "  make fmt          Format code"
	@echo "  make ci           Run CI checks (fmt, lint, test)"
	@echo "  make clean        Remove build artifacts and database"
	@echo ""
	@echo "Environment Variables:"
	@echo "  JOBS_DIR          Directory containing job YAML files (default: $(JOBS_DIR))"
	@echo "  DB_FILE           SQLite database file path (default: $(DB_FILE))"

# Run all checks (formatting, linting, tests)
all: fmt-check lint test

# Quick compile check without producing binaries
check:
	cargo check --all-features

# Format code
fmt:
	cargo fmt

# Check formatting without modifying files
fmt-check:
	cargo fmt -- --check

# Run clippy linter with warnings as errors
lint:
	cargo clippy --all-features --all-targets -- -D warnings

# Run tests
test:
	cargo test --all-features

# Build targets
build:
	cargo build --release

build-tui:
	cargo build --release --features tui

# Run targets
run:
	cargo run -- run $(JOBS_DIR)

run-sqlite: build-tui
	cargo run --features sqlite -- run $(JOBS_DIR) --db $(DB_FILE)

tui: build-tui
	cargo run --features tui --bin petit-tui -- --db $(DB_FILE)

# Clean build artifacts
clean:
	cargo clean
	rm -f $(DB_FILE)

# Run CI checks (same as GitHub Actions)
ci: fmt-check lint test
