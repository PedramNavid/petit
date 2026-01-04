//! Event handlers for the scheduler.
//!
//! This module contains event handlers that update task states in storage
//! and forward events to other event buses.

use std::sync::Arc;

use crate::core::types::RunId;
use crate::events::{Event, EventBus, EventHandler};
use crate::storage::Storage;

/// Event handler that updates task states in storage as tasks execute.
pub(crate) struct TaskStateUpdater<S: Storage> {
    pub(crate) storage: Arc<S>,
    pub(crate) run_id: RunId,
}

#[async_trait::async_trait]
impl<S: Storage + 'static> EventHandler for TaskStateUpdater<S> {
    async fn handle(&self, event: &Event) {
        match event {
            Event::TaskStarted { task_id, .. } => {
                if let Ok(mut state) = self.storage.get_task_state(&self.run_id, task_id).await {
                    state.mark_running();
                    if let Err(e) = self.storage.update_task_state(state).await {
                        tracing::warn!(task_id = %task_id, run_id = %self.run_id, error = %e, "Failed to update task state to running");
                    }
                }
            }
            Event::TaskCompleted { task_id, .. } => {
                if let Ok(mut state) = self.storage.get_task_state(&self.run_id, task_id).await {
                    state.mark_completed();
                    if let Err(e) = self.storage.update_task_state(state).await {
                        tracing::warn!(task_id = %task_id, run_id = %self.run_id, error = %e, "Failed to update task state to completed");
                    }
                }
            }
            Event::TaskFailed { task_id, error, .. } => {
                if let Ok(mut state) = self.storage.get_task_state(&self.run_id, task_id).await {
                    state.mark_failed(error);
                    if let Err(e) = self.storage.update_task_state(state).await {
                        tracing::warn!(task_id = %task_id, run_id = %self.run_id, error = %e, "Failed to update task state to failed");
                    }
                }
            }
            _ => {}
        }
    }
}

/// Event handler that forwards events to another event bus.
pub(crate) struct EventForwarder {
    pub(crate) target: Arc<EventBus>,
}

#[async_trait::async_trait]
impl EventHandler for EventForwarder {
    async fn handle(&self, event: &Event) {
        self.target.emit(event.clone()).await;
    }
}
