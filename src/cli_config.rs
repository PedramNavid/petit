//! CLI configuration file support.
//!
//! This module provides support for loading configuration from TOML files.
//! Configuration can be loaded from:
//! 1. An explicit path specified via --config flag
//! 2. The XDG config directory (~/.config/petit/config.toml)
//! 3. Fall back to defaults

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Failed to parse TOML config: {0}")]
    TomlError(#[from] toml::de::Error),
}

/// Storage configuration options.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(tag = "engine", rename_all = "lowercase")]
pub enum StorageConfig {
    /// In-memory storage (default).
    #[default]
    Memory,
    /// SQLite storage with a database file path.
    #[cfg(feature = "sqlite")]
    Sqlite { path: PathBuf },
}

/// API server configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApiConfig {
    /// Enable the API server (default: true).
    #[serde(default = "default_api_enabled")]
    pub enabled: bool,

    /// API server host (default: 127.0.0.1).
    #[serde(default = "default_api_host")]
    pub host: String,

    /// API server port (default: 8565).
    #[serde(default = "default_api_port")]
    pub port: u16,
}

fn default_api_enabled() -> bool {
    true
}

fn default_api_host() -> String {
    "127.0.0.1".to_string()
}

fn default_api_port() -> u16 {
    8565
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            enabled: default_api_enabled(),
            host: default_api_host(),
            port: default_api_port(),
        }
    }
}

/// Scheduler configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SchedulerConfig {
    /// Maximum concurrent jobs (default: unlimited).
    #[serde(default)]
    pub max_jobs: Option<usize>,

    /// Maximum concurrent tasks per job (default: 4).
    #[serde(default = "default_max_tasks")]
    pub max_tasks: usize,

    /// Scheduler tick interval in seconds (default: 1).
    #[serde(default = "default_tick_interval")]
    pub tick_interval: u64,
}

fn default_max_tasks() -> usize {
    4
}

fn default_tick_interval() -> u64 {
    1
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            max_jobs: None,
            max_tasks: default_max_tasks(),
            tick_interval: default_tick_interval(),
        }
    }
}

/// Top-level configuration structure.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Config {
    /// Storage configuration.
    #[serde(default)]
    pub storage: StorageConfig,

    /// API server configuration.
    #[cfg(feature = "api")]
    #[serde(default)]
    pub api: ApiConfig,

    /// Scheduler configuration.
    #[serde(default)]
    pub scheduler: SchedulerConfig,
}

impl Config {
    /// Load configuration from a TOML file.
    pub fn from_file(path: &PathBuf) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&contents)?;
        Ok(config)
    }

    /// Get the default XDG config path (~/.config/petit/config.toml).
    pub fn default_config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|mut path| {
            path.push("petit");
            path.push("config.toml");
            path
        })
    }

    /// Load configuration with priority:
    /// 1. Explicit config path if provided
    /// 2. XDG config path if it exists
    /// 3. Default configuration
    pub fn load(explicit_path: Option<PathBuf>) -> Result<Self, ConfigError> {
        // Try explicit path first
        if let Some(path) = explicit_path {
            return Self::from_file(&path);
        }

        // Try XDG default path
        if let Some(path) = Self::default_config_path()
            && path.exists()
        {
            return Self::from_file(&path);
        }

        // Fall back to defaults
        Ok(Self::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert!(matches!(config.storage, StorageConfig::Memory));
        #[cfg(feature = "api")]
        {
            assert!(config.api.enabled);
            assert_eq!(config.api.host, "127.0.0.1");
            assert_eq!(config.api.port, 8565);
        }
        assert_eq!(config.scheduler.max_tasks, 4);
        assert_eq!(config.scheduler.tick_interval, 1);
    }

    #[test]
    fn test_parse_memory_storage() {
        let toml = r#"
[storage]
engine = "memory"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(matches!(config.storage, StorageConfig::Memory));
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_parse_sqlite_storage() {
        let toml = r#"
[storage]
engine = "sqlite"
path = "/tmp/test.db"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        match config.storage {
            StorageConfig::Sqlite { path } => {
                assert_eq!(path, PathBuf::from("/tmp/test.db"));
            }
            _ => panic!("Expected SQLite storage"),
        }
    }

    #[cfg(feature = "api")]
    #[test]
    fn test_parse_api_config() {
        let toml = r#"
[api]
enabled = false
host = "0.0.0.0"
port = 9000
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(!config.api.enabled);
        assert_eq!(config.api.host, "0.0.0.0");
        assert_eq!(config.api.port, 9000);
    }

    #[test]
    fn test_parse_scheduler_config() {
        let toml = r#"
[scheduler]
max_jobs = 10
max_tasks = 8
tick_interval = 5
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.scheduler.max_jobs, Some(10));
        assert_eq!(config.scheduler.max_tasks, 8);
        assert_eq!(config.scheduler.tick_interval, 5);
    }

    #[test]
    fn test_parse_full_config() {
        let toml = r#"
[storage]
engine = "memory"

[scheduler]
max_jobs = 5
max_tasks = 2
tick_interval = 10
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(matches!(config.storage, StorageConfig::Memory));
        assert_eq!(config.scheduler.max_jobs, Some(5));
        assert_eq!(config.scheduler.max_tasks, 2);
        assert_eq!(config.scheduler.tick_interval, 10);
    }
}
