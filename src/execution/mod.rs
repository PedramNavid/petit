//! Task execution engine.
//!
//! This module provides the execution infrastructure for running tasks,
//! including external command execution and task orchestration.

mod command;
mod executor;

pub use command::{CommandTask, CommandTaskBuilder};
pub use executor::{TaskExecutor, TaskResult};
