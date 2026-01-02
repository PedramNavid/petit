//! Scheduler engine for job execution.
//!
//! This module provides the main scheduling loop that triggers jobs
//! at scheduled times and handles recovery from interruptions.

mod engine;

pub use engine::{Scheduler, SchedulerError, SchedulerHandle, SchedulerState};
