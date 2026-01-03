//! Configuration loading and parsing.
//!
//! This module provides YAML-based configuration for jobs and global settings.

mod builder;
mod yaml;

pub use builder::{JobConfigBuilder, load_jobs_from_directory};
pub use yaml::{
    ConfigError, GlobalConfig, JobConfig, RetryConfig, ScheduleConfig, TaskConfig, YamlLoader,
};
