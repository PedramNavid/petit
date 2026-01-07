//! petit - A minimal, lightweight task orchestrator.
//!
//! Usage:
//!   pt run <jobs-dir>     Run the scheduler with jobs from the specified directory
//!   pt validate <jobs-dir> Validate job configurations without running
//!   pt list <jobs-dir>    List all jobs in the directory

mod cli_config;

use clap::{Parser, Subcommand};
use cli_config::Config;
use petit::{
    DagExecutor, EventBus, EventHandler, InMemoryStorage, Scheduler, Storage,
    load_jobs_from_directory,
};

#[cfg(feature = "api")]
use petit::{ApiConfig, create_api_state, start_server};

#[cfg(feature = "sqlite")]
use petit::SqliteStorage;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};

/// pt - A minimal, lightweight task orchestrator
#[derive(Parser)]
#[command(name = "pt")]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Path to configuration file (overrides XDG default)
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the scheduler with jobs from a directory
    Run {
        /// Path to the directory containing job YAML files
        #[arg(value_name = "JOBS_DIR")]
        jobs_dir: PathBuf,

        /// Maximum concurrent jobs (overrides config file)
        #[arg(short = 'j', long)]
        max_jobs: Option<usize>,

        /// Maximum concurrent tasks per job (overrides config file, default: 4)
        #[arg(short = 't', long)]
        max_tasks: Option<usize>,

        /// Scheduler tick interval in seconds (overrides config file, default: 1)
        #[arg(long)]
        tick_interval: Option<u64>,

        /// Path to SQLite database file for persistent storage (overrides config file)
        #[cfg(feature = "sqlite")]
        #[arg(long, env = "PETIT_DB")]
        db: Option<PathBuf>,

        /// Disable the HTTP API server
        #[cfg(feature = "api")]
        #[arg(long)]
        no_api: bool,

        /// API server port (overrides config file, default: 8565)
        #[cfg(feature = "api")]
        #[arg(long)]
        api_port: Option<u16>,

        /// API server host (overrides config file, default: 127.0.0.1)
        #[cfg(feature = "api")]
        #[arg(long)]
        api_host: Option<String>,
    },

    /// Validate job configurations without running
    Validate {
        /// Path to the directory containing job YAML files
        #[arg(value_name = "JOBS_DIR")]
        jobs_dir: PathBuf,
    },

    /// List all jobs in the directory
    List {
        /// Path to the directory containing job YAML files
        #[arg(value_name = "JOBS_DIR")]
        jobs_dir: PathBuf,
    },

    /// Trigger a job manually (one-shot execution)
    Trigger {
        /// Path to the directory containing job YAML files
        #[arg(value_name = "JOBS_DIR")]
        jobs_dir: PathBuf,

        /// Job ID to trigger
        #[arg(value_name = "JOB_ID")]
        job_id: String,

        /// Path to SQLite database file for persistent storage (requires sqlite feature)
        #[cfg(feature = "sqlite")]
        #[arg(long, env = "PETIT_DB")]
        db: Option<PathBuf>,
    },
}

/// Merge CLI arguments with file configuration (with API support).
/// CLI arguments take priority over file configuration.
#[cfg(feature = "api")]
#[allow(clippy::too_many_arguments)]
fn merge_config(
    file_config: &Config,
    cli_max_jobs: Option<usize>,
    cli_max_tasks: Option<usize>,
    cli_tick_interval: Option<u64>,
    cli_db: Option<PathBuf>,
    cli_no_api: bool,
    cli_api_port: Option<u16>,
    cli_api_host: Option<String>,
) -> (
    Option<usize>,     // max_jobs
    usize,             // max_tasks
    u64,               // tick_interval
    Option<PathBuf>,   // db_path
    Option<ApiConfig>, // api_config
) {
    // Merge max_jobs: CLI > config file > default (None)
    let max_jobs = cli_max_jobs.or(file_config.scheduler.max_jobs);

    // Merge max_tasks: CLI > config file > default
    let max_tasks = cli_max_tasks.unwrap_or(file_config.scheduler.max_tasks);

    // Merge tick_interval: CLI > config file > default
    let tick_interval = cli_tick_interval.unwrap_or(file_config.scheduler.tick_interval);

    // Merge storage configuration
    let db_path = cli_db.or_else(|| {
        #[cfg(feature = "sqlite")]
        if let cli_config::StorageConfig::Sqlite { path } = &file_config.storage {
            return Some(path.clone());
        }
        None
    });

    // Merge API configuration
    // API is enabled if: not explicitly disabled via --no-api AND
    // (api.enabled in config OR any CLI API flag is set)
    let api_config = if cli_no_api {
        None
    } else {
        let cli_api_flags_set = cli_api_host.is_some() || cli_api_port.is_some();
        let api_enabled = file_config.api.enabled || cli_api_flags_set;
        if api_enabled {
            let host = cli_api_host.unwrap_or_else(|| file_config.api.host.clone());
            let port = cli_api_port.unwrap_or(file_config.api.port);
            Some(ApiConfig::new(host, port))
        } else {
            None
        }
    };

    (max_jobs, max_tasks, tick_interval, db_path, api_config)
}

/// Simple logging event handler that prints job events.
struct LoggingHandler;

#[async_trait::async_trait]
impl EventHandler for LoggingHandler {
    async fn handle(&self, event: &petit::Event) {
        match event {
            petit::Event::JobStarted { job_id, run_id, .. } => {
                info!("Job '{}' started (run: {})", job_id, run_id);
            }
            petit::Event::JobCompleted {
                job_id,
                run_id,
                success,
                duration,
                ..
            } => {
                if *success {
                    info!(
                        "Job '{}' completed successfully in {:?} (run: {})",
                        job_id, duration, run_id
                    );
                } else {
                    error!(
                        "Job '{}' failed after {:?} (run: {})",
                        job_id, duration, run_id
                    );
                }
            }
            petit::Event::TaskStarted { task_id, .. } => {
                info!("  Task '{}' started", task_id);
            }
            petit::Event::TaskCompleted {
                task_id,
                stdout,
                stderr,
                exit_code,
                ..
            } => {
                let exit_info = exit_code
                    .map(|c| format!(" (exit: {})", c))
                    .unwrap_or_default();
                info!("  Task '{}' completed{}", task_id, exit_info);
                if let Some(out) = stdout {
                    let out = out.trim();
                    if !out.is_empty() {
                        for line in out.lines() {
                            info!("    stdout: {}", line);
                        }
                    }
                }
                if let Some(err) = stderr {
                    let err = err.trim();
                    if !err.is_empty() {
                        for line in err.lines() {
                            warn!("    stderr: {}", line);
                        }
                    }
                }
            }
            petit::Event::TaskFailed {
                task_id,
                error,
                stdout,
                stderr,
                exit_code,
                ..
            } => {
                let exit_info = exit_code
                    .map(|c| format!(" (exit: {})", c))
                    .unwrap_or_default();
                warn!("  Task '{}' failed{}: {}", task_id, exit_info, error);
                if let Some(out) = stdout {
                    let out = out.trim();
                    if !out.is_empty() {
                        for line in out.lines() {
                            info!("    stdout: {}", line);
                        }
                    }
                }
                if let Some(err) = stderr {
                    let err = err.trim();
                    if !err.is_empty() {
                        for line in err.lines() {
                            error!("    stderr: {}", line);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let cli = Cli::parse();

    // Load configuration from file (explicit or XDG default)
    let file_config = match Config::load(cli.config.clone()) {
        Ok(config) => {
            if cli.config.is_some() {
                info!(
                    "Loaded configuration from: {}",
                    cli.config.as_ref().unwrap().display()
                );
            } else if let Some(default_path) = Config::default_config_path()
                && default_path.exists()
            {
                info!("Loaded configuration from: {}", default_path.display());
            }
            config
        }
        Err(e) => {
            error!("Failed to load configuration: {}", e);
            return Err(e.into());
        }
    };

    match cli.command {
        #[cfg(all(feature = "sqlite", feature = "api"))]
        Commands::Run {
            jobs_dir,
            max_jobs,
            max_tasks,
            tick_interval,
            db,
            no_api,
            api_port,
            api_host,
        } => {
            let (
                merged_max_jobs,
                merged_max_tasks,
                merged_tick_interval,
                merged_db,
                merged_api_config,
            ) = merge_config(
                &file_config,
                max_jobs,
                max_tasks,
                tick_interval,
                db,
                no_api,
                api_port,
                api_host,
            );
            run_scheduler(
                jobs_dir,
                merged_max_jobs,
                merged_max_tasks,
                merged_tick_interval,
                merged_db,
                merged_api_config,
            )
            .await?;
        }
        #[cfg(all(feature = "sqlite", not(feature = "api")))]
        Commands::Run {
            jobs_dir,
            max_jobs,
            max_tasks,
            tick_interval,
            db,
        } => {
            let (merged_max_jobs, merged_max_tasks, merged_tick_interval, merged_db) = {
                let config_max_jobs = max_jobs.or(file_config.scheduler.max_jobs);
                let config_max_tasks = max_tasks.unwrap_or(file_config.scheduler.max_tasks);
                let config_tick_interval =
                    tick_interval.unwrap_or(file_config.scheduler.tick_interval);
                let config_db = db.or_else(|| {
                    if let cli_config::StorageConfig::Sqlite { path } = &file_config.storage {
                        Some(path.clone())
                    } else {
                        None
                    }
                });
                (
                    config_max_jobs,
                    config_max_tasks,
                    config_tick_interval,
                    config_db,
                )
            };
            run_scheduler(
                jobs_dir,
                merged_max_jobs,
                merged_max_tasks,
                merged_tick_interval,
                merged_db,
                (),
            )
            .await?;
        }
        #[cfg(all(not(feature = "sqlite"), feature = "api"))]
        Commands::Run {
            jobs_dir,
            max_jobs,
            max_tasks,
            tick_interval,
            no_api,
            api_port,
            api_host,
        } => {
            let (merged_max_jobs, merged_max_tasks, merged_tick_interval, _, merged_api_config) =
                merge_config(
                    &file_config,
                    max_jobs,
                    max_tasks,
                    tick_interval,
                    None,
                    no_api,
                    api_port,
                    api_host,
                );
            run_scheduler(
                jobs_dir,
                merged_max_jobs,
                merged_max_tasks,
                merged_tick_interval,
                None::<PathBuf>,
                merged_api_config,
            )
            .await?;
        }
        #[cfg(all(not(feature = "sqlite"), not(feature = "api")))]
        Commands::Run {
            jobs_dir,
            max_jobs,
            max_tasks,
            tick_interval,
        } => {
            let (merged_max_jobs, merged_max_tasks, merged_tick_interval, _) = {
                let config_max_jobs = max_jobs.or(file_config.scheduler.max_jobs);
                let config_max_tasks = max_tasks.unwrap_or(file_config.scheduler.max_tasks);
                let config_tick_interval =
                    tick_interval.unwrap_or(file_config.scheduler.tick_interval);
                (
                    config_max_jobs,
                    config_max_tasks,
                    config_tick_interval,
                    None::<PathBuf>,
                )
            };
            run_scheduler(
                jobs_dir,
                merged_max_jobs,
                merged_max_tasks,
                merged_tick_interval,
                None::<PathBuf>,
                (),
            )
            .await?;
        }
        Commands::Validate { jobs_dir } => {
            validate_jobs(jobs_dir)?;
        }
        Commands::List { jobs_dir } => {
            list_jobs(jobs_dir)?;
        }
        #[cfg(feature = "sqlite")]
        Commands::Trigger {
            jobs_dir,
            job_id,
            db,
        } => {
            // Merge CLI db with config file storage setting
            let merged_db = db.or_else(|| {
                if let cli_config::StorageConfig::Sqlite { path } = &file_config.storage {
                    Some(path.clone())
                } else {
                    None
                }
            });
            trigger_job(jobs_dir, job_id, merged_db).await?;
        }
        #[cfg(not(feature = "sqlite"))]
        Commands::Trigger { jobs_dir, job_id } => {
            trigger_job(jobs_dir, job_id, None::<PathBuf>).await?;
        }
    }

    Ok(())
}

/// Run the scheduler with jobs from a directory.
#[cfg(feature = "api")]
async fn run_scheduler(
    jobs_dir: PathBuf,
    max_jobs: Option<usize>,
    max_tasks: usize,
    tick_interval: u64,
    db_path: Option<PathBuf>,
    api_config: Option<ApiConfig>,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Loading jobs from: {}", jobs_dir.display());

    let jobs = load_jobs_from_directory(&jobs_dir)?;

    if jobs.is_empty() {
        warn!("No job files found in {}", jobs_dir.display());
        return Ok(());
    }

    info!("Loaded {} job(s):", jobs.len());
    for job in &jobs {
        let schedule_info = if job.is_scheduled() {
            "scheduled"
        } else {
            "manual only"
        };
        let enabled_info = if job.is_enabled() { "" } else { " (disabled)" };
        info!(
            "  - {} ({}){}: {} task(s)",
            job.id(),
            schedule_info,
            enabled_info,
            job.dag().len()
        );
    }

    // Create event bus with logging handler
    let event_bus = EventBus::new();
    event_bus.register(Arc::new(LoggingHandler)).await;

    // Create DAG executor
    let dag_executor = DagExecutor::with_concurrency(max_tasks);

    // Create storage and scheduler based on db_path
    #[cfg(feature = "sqlite")]
    let scheduler_task = if let Some(db) = db_path {
        info!("Using SQLite storage: {}", db.display());
        let storage = Arc::new(SqliteStorage::new(&db).await?);
        run_scheduler_with_storage(
            storage,
            jobs,
            event_bus,
            dag_executor,
            tick_interval,
            max_jobs,
            api_config,
        )
        .await
    } else {
        info!("Using in-memory storage");
        let storage = Arc::new(InMemoryStorage::new());
        run_scheduler_with_storage(
            storage,
            jobs,
            event_bus,
            dag_executor,
            tick_interval,
            max_jobs,
            api_config,
        )
        .await
    };

    #[cfg(not(feature = "sqlite"))]
    let scheduler_task = {
        let _ = db_path; // suppress unused warning
        info!("Using in-memory storage");
        let storage = Arc::new(InMemoryStorage::new());
        run_scheduler_with_storage(
            storage,
            jobs,
            event_bus,
            dag_executor,
            tick_interval,
            max_jobs,
            api_config,
        )
        .await
    };

    scheduler_task
}

/// Run the scheduler with jobs from a directory (no API).
#[cfg(not(feature = "api"))]
async fn run_scheduler(
    jobs_dir: PathBuf,
    max_jobs: Option<usize>,
    max_tasks: usize,
    tick_interval: u64,
    db_path: Option<PathBuf>,
    _api_config: (),
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Loading jobs from: {}", jobs_dir.display());

    let jobs = load_jobs_from_directory(&jobs_dir)?;

    if jobs.is_empty() {
        warn!("No job files found in {}", jobs_dir.display());
        return Ok(());
    }

    info!("Loaded {} job(s):", jobs.len());
    for job in &jobs {
        let schedule_info = if job.is_scheduled() {
            "scheduled"
        } else {
            "manual only"
        };
        let enabled_info = if job.is_enabled() { "" } else { " (disabled)" };
        info!(
            "  - {} ({}){}: {} task(s)",
            job.id(),
            schedule_info,
            enabled_info,
            job.dag().len()
        );
    }

    // Create event bus with logging handler
    let event_bus = EventBus::new();
    event_bus.register(Arc::new(LoggingHandler)).await;

    // Create DAG executor
    let dag_executor = DagExecutor::with_concurrency(max_tasks);

    // Create storage and scheduler based on db_path
    #[cfg(feature = "sqlite")]
    let scheduler_task = if let Some(db) = db_path {
        info!("Using SQLite storage: {}", db.display());
        let storage = Arc::new(SqliteStorage::new(&db).await?);
        run_scheduler_with_storage_no_api(
            storage,
            jobs,
            event_bus,
            dag_executor,
            tick_interval,
            max_jobs,
        )
        .await
    } else {
        info!("Using in-memory storage");
        let storage = Arc::new(InMemoryStorage::new());
        run_scheduler_with_storage_no_api(
            storage,
            jobs,
            event_bus,
            dag_executor,
            tick_interval,
            max_jobs,
        )
        .await
    };

    #[cfg(not(feature = "sqlite"))]
    let scheduler_task = {
        let _ = db_path; // suppress unused warning
        info!("Using in-memory storage");
        let storage = Arc::new(InMemoryStorage::new());
        run_scheduler_with_storage_no_api(
            storage,
            jobs,
            event_bus,
            dag_executor,
            tick_interval,
            max_jobs,
        )
        .await
    };

    scheduler_task
}

/// Helper to run scheduler with any storage type (with API support).
#[cfg(feature = "api")]
async fn run_scheduler_with_storage<S: Storage + 'static>(
    storage: Arc<S>,
    jobs: Vec<petit::Job>,
    event_bus: EventBus,
    dag_executor: DagExecutor,
    tick_interval: u64,
    max_jobs: Option<usize>,
    api_config: Option<ApiConfig>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Create scheduler with shared storage
    let mut scheduler = Scheduler::with_storage(Arc::clone(&storage))
        .with_event_bus(event_bus)
        .with_dag_executor(dag_executor)
        .with_tick_interval(Duration::from_secs(tick_interval));

    if let Some(max) = max_jobs {
        scheduler = scheduler.with_max_concurrent_jobs(max);
    }

    // Clone jobs for API state before registering (registration consumes them)
    let jobs_for_api = jobs.clone();

    // Register all jobs
    for job in jobs {
        scheduler.register(job);
    }

    // Start the scheduler
    info!("Starting scheduler (tick interval: {}s)...", tick_interval);
    info!("Press Ctrl+C to stop");

    let (handle, scheduler_task) = scheduler.start().await;

    // Start API server if configured
    let _api_handle = if let Some(config) = api_config {
        let api_state = create_api_state(handle.clone(), storage, jobs_for_api);
        Some(start_server(config, api_state).await?)
    } else {
        None
    };

    // Wait for Ctrl+C
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("\nShutting down...");
            handle.shutdown().await?;
        }
        _ = scheduler_task => {
            info!("Scheduler stopped");
        }
    }

    info!("Goodbye!");
    Ok(())
}

/// Helper to run scheduler with any storage type (no API).
#[cfg(not(feature = "api"))]
async fn run_scheduler_with_storage_no_api<S: Storage + 'static>(
    storage: Arc<S>,
    jobs: Vec<petit::Job>,
    event_bus: EventBus,
    dag_executor: DagExecutor,
    tick_interval: u64,
    max_jobs: Option<usize>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Create scheduler with shared storage
    let mut scheduler = Scheduler::with_storage(storage)
        .with_event_bus(event_bus)
        .with_dag_executor(dag_executor)
        .with_tick_interval(Duration::from_secs(tick_interval));

    if let Some(max) = max_jobs {
        scheduler = scheduler.with_max_concurrent_jobs(max);
    }

    // Register all jobs
    for job in jobs {
        scheduler.register(job);
    }

    // Start the scheduler
    info!("Starting scheduler (tick interval: {}s)...", tick_interval);
    info!("Press Ctrl+C to stop");

    let (handle, scheduler_task) = scheduler.start().await;

    // Wait for Ctrl+C
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("\nShutting down...");
            handle.shutdown().await?;
        }
        _ = scheduler_task => {
            info!("Scheduler stopped");
        }
    }

    info!("Goodbye!");
    Ok(())
}

/// Validate job configurations without running.
fn validate_jobs(jobs_dir: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    info!("Validating jobs in: {}", jobs_dir.display());

    match load_jobs_from_directory(&jobs_dir) {
        Ok(jobs) => {
            info!("All {} job(s) are valid:", jobs.len());
            for job in &jobs {
                info!("  - {} ({}): OK", job.id(), job.name());
            }
            Ok(())
        }
        Err(e) => {
            error!("Validation failed: {}", e);
            Err(e.into())
        }
    }
}

/// List all jobs in the directory.
fn list_jobs(jobs_dir: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let jobs = load_jobs_from_directory(&jobs_dir)?;

    if jobs.is_empty() {
        println!("No jobs found in {}", jobs_dir.display());
        return Ok(());
    }

    println!("Jobs in {}:", jobs_dir.display());
    println!();

    for job in &jobs {
        println!("ID: {}", job.id());
        println!("  Name: {}", job.name());
        println!("  Enabled: {}", job.is_enabled());
        println!(
            "  Schedule: {}",
            if job.is_scheduled() {
                "yes"
            } else {
                "manual only"
            }
        );
        println!("  Tasks: {}", job.dag().len());

        // List tasks
        if let Ok(order) = job.dag().topological_sort() {
            for task_id in &order {
                let deps = job.dag().get_dependencies(task_id).unwrap_or(&[]);
                if deps.is_empty() {
                    println!("    - {}", task_id);
                } else {
                    let dep_names: Vec<&str> = deps.iter().map(|d| d.as_str()).collect();
                    println!("    - {} (depends on: {})", task_id, dep_names.join(", "));
                }
            }
        }

        if let Some(max) = job.max_concurrency() {
            println!("  Max concurrent runs: {}", max);
        }

        println!();
    }

    Ok(())
}

/// Event handler that signals when a specific job completes.
struct CompletionWatcher {
    target_job_id: String,
    completed: Arc<tokio::sync::Notify>,
}

#[async_trait::async_trait]
impl EventHandler for CompletionWatcher {
    async fn handle(&self, event: &petit::Event) {
        if let petit::Event::JobCompleted { job_id, .. } = event
            && job_id.as_str() == self.target_job_id
        {
            self.completed.notify_one();
        }
    }
}

/// Trigger a specific job and wait for it to complete.
async fn trigger_job(
    jobs_dir: PathBuf,
    job_id: String,
    db_path: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Loading jobs from: {}", jobs_dir.display());

    let jobs = load_jobs_from_directory(&jobs_dir)?;

    // Check if the job exists
    if !jobs.iter().any(|j| j.id().as_str() == job_id) {
        error!("Job '{}' not found", job_id);
        error!(
            "Available jobs: {}",
            jobs.iter()
                .map(|j| j.id().as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        return Err(format!("Job '{}' not found", job_id).into());
    }

    // Create event bus with logging and completion watching handlers
    let event_bus = EventBus::new();
    event_bus.register(Arc::new(LoggingHandler)).await;

    // Create completion watcher before starting the scheduler
    let completed = Arc::new(tokio::sync::Notify::new());
    let watcher = CompletionWatcher {
        target_job_id: job_id.clone(),
        completed: completed.clone(),
    };
    event_bus.register(Arc::new(watcher)).await;

    // Create storage and scheduler based on db_path
    #[cfg(feature = "sqlite")]
    if let Some(db) = db_path {
        info!("Using SQLite storage: {}", db.display());
        let storage = Arc::new(SqliteStorage::new(&db).await?);
        return trigger_job_with_storage(storage, jobs, event_bus, job_id, completed).await;
    }

    #[cfg(not(feature = "sqlite"))]
    let _ = db_path; // suppress unused warning

    info!("Using in-memory storage");
    let storage = Arc::new(InMemoryStorage::new());
    trigger_job_with_storage(storage, jobs, event_bus, job_id, completed).await
}

/// Helper to trigger a job with any storage type.
async fn trigger_job_with_storage<S: Storage + 'static>(
    storage: Arc<S>,
    jobs: Vec<petit::Job>,
    event_bus: EventBus,
    job_id: String,
    completed: Arc<tokio::sync::Notify>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut scheduler = Scheduler::with_storage(Arc::clone(&storage)).with_event_bus(event_bus);

    // Register all jobs (needed for dependency resolution)
    for job in jobs {
        scheduler.register(job);
    }

    // Start the scheduler
    let (handle, _scheduler_task) = scheduler.start().await;

    // Trigger the job
    info!("Triggering job '{}'...", job_id);
    match handle.trigger(job_id.clone()).await {
        Ok(run_id) => {
            info!("Job triggered (run: {})", run_id);

            // Wait for completion with a timeout
            tokio::select! {
                _ = completed.notified() => {
                    // Job completed
                }
                _ = tokio::time::sleep(Duration::from_secs(300)) => {
                    warn!("Job timed out after 5 minutes");
                }
            }
        }
        Err(e) => {
            error!("Failed to trigger job: {}", e);
            handle.shutdown().await?;
            return Err(e.into());
        }
    }

    handle.shutdown().await?;
    info!("Done!");
    Ok(())
}
