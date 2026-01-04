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

    /// Failed to parse TOML.
    #[error("TOML parse error: {0}")]
    TomlError(toml::de::Error),

    /// Failed to parse TOML from a specific file.
    #[error("TOML parse error in '{path}': {source}")]
    TomlFileError {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    /// Invalid configuration value.
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    /// Missing required field.
    #[error("missing required field: {0}")]
    MissingField(String),
}
