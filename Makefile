# Petit Makefile
# Common targets for running the orchestrator and TUI

# Configuration
JOBS_DIR ?= examples/jobs
DB_FILE ?= petit.db

# Cross-compilation targets (musl for static linking)
TARGET_ARM64 := aarch64-unknown-linux-musl
TARGET_X86_64 := x86_64-unknown-linux-musl

# Cluster hosts
CLUSTER_HOSTS := cube1 cube2 cube3 cube4

# Output directories
DIST_DIR := dist

.PHONY: all check fmt fmt-check lint test bench build build-tui build-arm64 build-arm64-tui \
        build-all setup-cross deploy deploy-jobs install install-service start-service stop-service \
        restart-service status-service logs run run-sqlite tui clean ci help

help:
	@echo "Petit - Task Orchestrator"
	@echo ""
	@echo "Usage:"
	@echo "  make build            Build the petit binary (release, native)"
	@echo "  make build-tui        Build with TUI support (includes sqlite)"
	@echo "  make build-arm64      Cross-compile for Linux ARM64 (aarch64)"
	@echo "  make build-arm64-tui  Cross-compile with TUI for Linux ARM64"
	@echo "  make build-all        Build for all platforms"
	@echo "  make install          Install pt and petit-tui to ~/.cargo/bin"
	@echo "  make setup-cross      Install cross-compilation toolchain"
	@echo "  make deploy           Deploy pt and petit-tui to cluster hosts (cube1-cube4)"
	@echo "  make deploy-jobs      Deploy job YAML files to cluster hosts"
	@echo "  make install-service  Install systemd service on cluster hosts"
	@echo "  make start-service    Start petit on all cluster hosts"
	@echo "  make stop-service     Stop petit on all cluster hosts"
	@echo "  make restart-service  Restart petit on all cluster hosts"
	@echo "  make status-service   Show service status on all hosts"
	@echo "  make logs HOST=cube1  Tail logs from a specific host"
	@echo ""
	@echo "  make run              Run scheduler with in-memory storage"
	@echo "  make run-sqlite       Run scheduler with SQLite storage (DB_FILE=$(DB_FILE))"
	@echo "  make tui              Run the TUI dashboard (requires run-sqlite in another terminal)"
	@echo ""
	@echo "  make test             Run all tests"
	@echo "  make bench            Run benchmarks"
	@echo "  make lint             Run clippy linter"
	@echo "  make fmt              Format code"
	@echo "  make ci               Run CI checks (fmt, lint, test)"
	@echo "  make clean            Remove build artifacts and database"
	@echo ""
	@echo "Environment Variables:"
	@echo "  JOBS_DIR              Directory containing job YAML files (default: $(JOBS_DIR))"
	@echo "  DB_FILE               SQLite database file path (default: $(DB_FILE))"
	@echo "  CLUSTER_HOSTS         Space-separated list of hosts (default: $(CLUSTER_HOSTS))"

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

# Run benchmarks
bench:
	cargo bench --all-features

# Build targets (native)
build:
	cargo build --release

build-tui:
	cargo build --release --features tui

# Cross-compilation targets for ARM64 Linux
build-arm64:
	cargo build --release --target $(TARGET_ARM64) --features sqlite
	@mkdir -p $(DIST_DIR)/$(TARGET_ARM64)
	cp target/$(TARGET_ARM64)/release/pt $(DIST_DIR)/$(TARGET_ARM64)/

build-arm64-tui:
	cargo build --release --target $(TARGET_ARM64) --features tui
	@mkdir -p $(DIST_DIR)/$(TARGET_ARM64)
	cp target/$(TARGET_ARM64)/release/pt $(DIST_DIR)/$(TARGET_ARM64)/
	cp target/$(TARGET_ARM64)/release/petit-tui $(DIST_DIR)/$(TARGET_ARM64)/

# Build for all platforms
build-all: build build-arm64

# Install binaries to ~/.cargo/bin
install:
	cargo install --path .
	cargo install --path . --bin petit-tui --features tui

# Setup cross-compilation toolchain
setup-cross:
	rustup target add $(TARGET_ARM64)
	@echo ""
	@echo "Rust target installed. You also need the musl cross-compiler:"
	@echo "  Arch Linux: yay -S aarch64-linux-musl (from AUR)"
	@echo ""
	@echo "The .cargo/config.toml is already configured for musl."

# Determine target hosts (single HOST or all CLUSTER_HOSTS)
TARGETS = $(if $(HOST),$(HOST),$(CLUSTER_HOSTS))

# Deploy to cluster hosts
deploy: build-arm64-tui
	@for host in $(TARGETS); do \
		echo "Deploying to $$host..."; \
		ssh $$host 'mkdir -p ~/bin ~/pt/jobs' && \
		scp $(DIST_DIR)/$(TARGET_ARM64)/pt $(DIST_DIR)/$(TARGET_ARM64)/petit-tui $$host:~/bin/; \
	done

# Install systemd service on cluster hosts
install-service: deploy
	@for host in $(TARGETS); do \
		echo "Installing service on $$host..."; \
		ssh $$host 'cat > /tmp/petit.service' < dist/petit.service && \
		ssh $$host 'sudo mv /tmp/petit.service /etc/systemd/system/petit.service && \
			sudo systemctl daemon-reload && \
			sudo systemctl enable petit'; \
	done
	@echo "Start with: make start-service"

# Service control targets
start-service:
	@for host in $(TARGETS); do \
		echo "Starting petit on $$host..."; \
		ssh $$host 'sudo systemctl start petit'; \
	done

stop-service:
	@for host in $(TARGETS); do \
		echo "Stopping petit on $$host..."; \
		ssh $$host 'sudo systemctl stop petit'; \
	done

restart-service:
	@for host in $(TARGETS); do \
		echo "Restarting petit on $$host..."; \
		ssh $$host 'sudo systemctl restart petit'; \
	done

status-service:
	@for host in $(TARGETS); do \
		echo "=== $$host ==="; \
		ssh $$host 'systemctl status petit --no-pager' || true; \
	done

logs:
	@if [ -z "$(HOST)" ]; then \
		echo "Usage: make logs HOST=cube1"; \
	else \
		ssh $(HOST) 'journalctl -u petit -f'; \
	fi

# Deploy job files to cluster
deploy-jobs:
	@for host in $(TARGETS); do \
		echo "Deploying jobs to $$host..."; \
		scp examples/cluster-jobs/*.yaml $$host:~/pt/jobs/; \
	done

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
	rm -rf $(DIST_DIR)/$(TARGET_ARM64)
	rm -rf $(DIST_DIR)/$(TARGET_X86_64)

# Run CI checks (same as GitHub Actions)
ci: fmt-check lint test
