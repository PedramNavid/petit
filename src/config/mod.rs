//! # YAML Configuration
//!
//! This module provides YAML-based configuration for jobs and global settings.
//!
//! # Security Considerations
//!
//! ## Secrets Management
//!
//! **Never hardcode secrets in YAML files.** Configuration files are typically stored in version
//! control and should not contain sensitive information like API keys, passwords, or tokens.
//!
//! ### Best Practices for Secrets
//!
//! 1. **Use environment variable references**: Reference environment variables that will be
//!    resolved at runtime:
//!
//!    ```yaml
//!    tasks:
//!      - id: api_call
//!        type: command
//!        command: curl
//!        args: ["-H", "Authorization: Bearer $API_TOKEN", "https://api.example.com"]
//!        environment:
//!          API_TOKEN: ${API_TOKEN}  # Resolved from system environment at runtime
//!    ```
//!
//! 2. **Keep secrets out of version control**: Add sensitive files to `.gitignore` and use
//!    separate configuration management for secrets (e.g., HashiCorp Vault, AWS Secrets Manager,
//!    or environment-specific `.env` files that are not committed).
//!
//!
//! ## Command Safety
//!
//! The config system allows execution of arbitrary commands, which poses security risks if
//! not handled properly.
//!
//! ### Command Injection Prevention
//!
//! Never interpolate untrusted input into command strings.
//! Always use the structured `args`
//! array for parameters instead of string concatenation:
//!
//! ```yaml
//! # UNSAFE: Vulnerable to command injection if $USER_INPUT comes from untrusted source
//! tasks:
//!   - id: bad_example
//!     type: command
//!     command: sh
//!     args: ["-c", "echo $USER_INPUT"]  # If USER_INPUT is from user, this is dangerous
//!
//! # SAFER: Use parameterized execution
//! tasks:
//!   - id: good_example
//!     type: command
//!     command: echo
//!     args: ["${USER_INPUT}"]  # Command arguments are passed separately, not interpreted by shell
//! ```
//!
//! ## Job Configuration
//!
//! Job definitions are written in YAML files with the `.yaml` or `.yml` extension.
//! Each file defines a single job with its tasks, dependencies, and scheduling information.
//!
//! ### Basic Job Example
//!
//! ```yaml
//! id: daily_etl
//! name: Daily ETL Pipeline
//! description: Extract, transform, and load data daily
//! schedule: "@daily"
//! tasks:
//!   - id: extract
//!     type: command
//!     command: python
//!     args: [extract.py]
//!   - id: transform
//!     type: command
//!     command: python
//!     args: [transform.py]
//!     depends_on: [extract]
//!   - id: load
//!     type: command
//!     command: python
//!     args: [load.py]
//!     depends_on: [transform]
//! ```
//!
//! ### Advanced Job Example with Retries and Timezones
//!
//! ```yaml
//! id: production_sync
//! name: Production Database Sync
//! schedule:
//!   cron: "0 0 * * *"
//!   timezone: "America/New_York"
//! max_concurrency: 1
//! tasks:
//!   - id: snapshot
//!     type: command
//!     command: ./snapshot.sh
//!     timeout_secs: 3600
//!     retry:
//!       max_attempts: 3
//!       delay_secs: 60
//!       condition: transient_only
//!   - id: sync
//!     type: command
//!     command: ./sync.sh
//!     depends_on: [snapshot]
//!     environment:
//!       DATABASE_URL: postgres://localhost/prod
//!   - id: cleanup
//!     type: command
//!     command: ./cleanup.sh
//!     depends_on: [sync]
//!     condition: all_done
//! ```
//!
//! ### Cross-Job Dependencies
//!
//! Jobs can depend on other jobs completing successfully:
//!
//! ```yaml
//! id: report_generation
//! name: Generate Reports
//! depends_on:
//!   - daily_etl  # Simple dependency - must succeed
//!   - job: data_validation
//!     condition: last_complete  # Must complete (success or failure)
//!   - job: data_refresh
//!     condition:
//!       within_window:
//!         seconds: 3600  # Must have succeeded within last hour
//! tasks:
//!   - id: generate
//!     type: command
//!     command: ./generate_report.sh
//! ```
//!
//! ## Global Configuration
//!
//! Global settings can be defined in `petit.yaml`:
//!
//! ```yaml
//! default_timezone: UTC
//! default_retry:
//!   max_attempts: 3
//!   delay_secs: 60
//! max_concurrent_jobs: 10
//! max_concurrent_tasks: 5
//! storage:
//!   type: sqlite
//!   path: /var/lib/petit/petit.db
//! environment:
//!   LOG_LEVEL: info
//!   ENVIRONMENT: production
//! ```
//!
//! ## Schedule Formats
//!
//! Schedules support both cron expressions and shortcuts:
//!
//! - `@hourly` - Run at the start of every hour
//! - `@daily` - Run at midnight every day
//! - `@weekly` - Run at midnight on Sunday
//! - `@monthly` - Run at midnight on the first of the month
//! - `"0 9 * * 1-5"` - Custom cron: 9 AM on weekdays
//!
//! ## Task Types
//!
//! ### Command Tasks
//!
//! Execute shell commands with arguments:
//!
//! ```yaml
//! tasks:
//!   - id: backup
//!     type: command
//!     command: /usr/local/bin/backup
//!     args: [--database, mydb, --output, /backups]
//!     working_dir: /app
//!     timeout_secs: 1800
//! ```
//!
//! ### Python Tasks
//!
//! Run Python scripts or inline code:
//!
//! ```yaml
//! tasks:
//!   - id: inline_python
//!     type: python
//!     script: |
//!       import requests
//!       response = requests.get('https://api.example.com/data')
//!       print(response.json())
//!     inline: true
//!
//!   - id: script_file
//!     type: python
//!     script: /app/scripts/process.py
//!     inline: false
//! ```
//!
//! ## Task Conditions
//!
//! Control when tasks run based on upstream task status:
//!
//! - `all_success` (default) - Run only if all upstream tasks succeeded
//! - `on_failure` - Run only if any upstream task failed
//! - `all_done` - Run regardless of upstream status (always runs)
//!
//! ```yaml
//! tasks:
//!   - id: main_task
//!     type: command
//!     command: ./main.sh
//!
//!   - id: on_success
//!     type: command
//!     command: ./notify_success.sh
//!     depends_on: [main_task]
//!     condition: all_success
//!
//!   - id: on_error
//!     type: command
//!     command: ./notify_error.sh
//!     depends_on: [main_task]
//!     condition: on_failure
//!
//!   - id: cleanup
//!     type: command
//!     command: ./cleanup.sh
//!     depends_on: [main_task]
//!     condition: all_done
//! ```
//!
//! ## Loading Configuration in Code
//!
//! ### Load a Single Job
//!
//! ```rust,no_run
//! use petit::config::{YamlLoader, JobConfigBuilder};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Load and parse YAML configuration
//! let config = YamlLoader::load_job_config("jobs/my_job.yaml")?;
//!
//! // Build a runnable Job from the configuration
//! let job = JobConfigBuilder::build(config)?;
//!
//! println!("Loaded job: {} ({})", job.name(), job.id());
//! # Ok(())
//! # }
//! ```
//!
//! ### Load All Jobs from Directory
//!
//! ```rust,no_run
//! use petit::config::load_jobs_from_directory;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Load all .yaml and .yml files from a directory
//! let jobs = load_jobs_from_directory("./jobs")?;
//!
//! println!("Loaded {} jobs", jobs.len());
//! for job in jobs {
//!     println!("  - {} ({})", job.name(), job.id());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Load Global Configuration
//!
//! ```rust,no_run
//! use petit::config::YamlLoader;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let global_config = YamlLoader::load_global_config("petit.yaml")?;
//!
//! if let Some(tz) = global_config.default_timezone {
//!     println!("Default timezone: {}", tz);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Validation
//!
//! Configuration is validated when loaded:
//!
//! - Job IDs and names must not be empty
//! - Jobs must have at least one task
//! - Task IDs must be unique within a job
//! - Task dependencies must reference existing tasks
//! - Max concurrency cannot be zero
//! - Cron expressions must be valid
//!
//! Invalid configurations will return a [`ConfigError`] with details about the issue.

mod builder;
mod error;
mod types;
mod yaml;

pub use builder::{JobConfigBuilder, load_jobs_from_directory};
pub use error::ConfigError;
pub use types::{
    GlobalConfig, JobConfig, JobDependencyConditionConfig, JobDependencyConfig,
    RetryConditionConfig, RetryConfig, ScheduleConfig, StorageConfig, TaskConditionConfig,
    TaskConfig, TaskTypeConfig,
};
pub use yaml::YamlLoader;
