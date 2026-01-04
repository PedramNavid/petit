//! Scheduler handle for controlling the scheduler.
//!
//! This module provides the `SchedulerHandle` type that allows external control
//! of the scheduler through commands like trigger, pause, resume, and shutdown.

use std::sync::Arc;

use tokio::sync::{RwLock, mpsc, oneshot};

use crate::core::types::{JobId, RunId};

use super::types::{SchedulerCommand, SchedulerError, SchedulerState};

/// Buffer size for the command channel between SchedulerHandle and Scheduler.
pub(crate) const COMMAND_CHANNEL_BUFFER: usize = 32;

/// Handle for controlling the scheduler.
#[derive(Clone)]
pub struct SchedulerHandle {
    pub(crate) command_tx: mpsc::Sender<SchedulerCommand>,
    pub(crate) state: Arc<RwLock<SchedulerState>>,
}

impl SchedulerHandle {
    /// Helper to send a command that returns a result and wait for response.
    async fn send_result_command<T>(
        &self,
        build_command: impl FnOnce(oneshot::Sender<Result<T, SchedulerError>>) -> SchedulerCommand,
        operation: &str,
    ) -> Result<T, SchedulerError>
    where
        T: Send + 'static,
    {
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx
            .send(build_command(response_tx))
            .await
            .map_err(|_| {
                SchedulerError::ChannelError(format!("failed to send {} command", operation))
            })?;

        response_rx.await.map_err(|_| {
            SchedulerError::ChannelError(format!("failed to receive {} response", operation))
        })?
    }

    /// Helper to send a command that returns unit and wait for response.
    async fn send_unit_command(
        &self,
        build_command: impl FnOnce(oneshot::Sender<()>) -> SchedulerCommand,
        operation: &str,
    ) -> Result<(), SchedulerError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx
            .send(build_command(response_tx))
            .await
            .map_err(|_| {
                SchedulerError::ChannelError(format!("failed to send {} command", operation))
            })?;

        response_rx.await.map_err(|_| {
            SchedulerError::ChannelError(format!("failed to receive {} response", operation))
        })?;

        Ok(())
    }

    /// Trigger a job manually.
    pub async fn trigger(&self, job_id: impl Into<JobId>) -> Result<RunId, SchedulerError> {
        let job_id = job_id.into();
        self.send_result_command(
            |response| SchedulerCommand::Trigger { job_id, response },
            "trigger",
        )
        .await
    }

    /// Pause the scheduler.
    ///
    /// While paused, scheduled jobs will not be triggered, but manual triggers still work.
    pub async fn pause(&self) -> Result<(), SchedulerError> {
        self.send_unit_command(|response| SchedulerCommand::Pause { response }, "pause")
            .await
    }

    /// Resume the scheduler after being paused.
    pub async fn resume(&self) -> Result<(), SchedulerError> {
        self.send_unit_command(|response| SchedulerCommand::Resume { response }, "resume")
            .await
    }

    /// Shutdown the scheduler.
    pub async fn shutdown(&self) -> Result<(), SchedulerError> {
        self.send_unit_command(
            |response| SchedulerCommand::Shutdown { response },
            "shutdown",
        )
        .await
    }

    /// Get the current scheduler state.
    pub async fn state(&self) -> SchedulerState {
        *self.state.read().await
    }

    /// Check if the scheduler is running.
    pub async fn is_running(&self) -> bool {
        *self.state.read().await == SchedulerState::Running
    }

    /// Check if the scheduler is paused.
    pub async fn is_paused(&self) -> bool {
        *self.state.read().await == SchedulerState::Paused
    }
}
