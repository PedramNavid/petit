//! Configuration error types.
//!
//! This module defines error types for configuration loading and validation.

use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur when loading configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Failed to read configuration file.
    #[error("failed to read file: {0}")]
    IoError(std::io::Error),

    /// Failed to read a specific file with context.
    #[error("failed to read file '{path}': {source}")]
    FileReadError {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Failed to read a directory with context.
    #[error("failed to read directory '{path}': {source}")]
    DirReadError {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Failed to parse YAML.
    #[error("YAML parse error: {0}")]
    YamlError(serde_yaml::Error),

    /// Failed to parse YAML from a specific file.
    #[error("YAML parse error in '{path}': {source}")]
    YamlFileError {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },

    /// Invalid configuration value.
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    /// Missing required field.
    #[error("missing required field: {0}")]
    MissingField(String),
}
