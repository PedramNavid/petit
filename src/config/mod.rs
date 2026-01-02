//! Configuration loading and parsing.
//!
//! This module provides YAML-based configuration for jobs and global settings.

mod yaml;

pub use yaml::{
    ConfigError, GlobalConfig, JobConfig, RetryConfig, ScheduleConfig, TaskConfig, YamlLoader,
};
