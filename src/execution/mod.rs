//! Task execution engine.
//!
//! This module provides the execution infrastructure for running tasks,
//! including external command execution.

mod command;

pub use command::{CommandTask, CommandTaskBuilder};
