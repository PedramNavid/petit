//! Integration tests for petit task orchestrator.
//!
//! These tests verify end-to-end scenarios including:
//! - Complete workflow from YAML to execution
//! - Recovery from interruptions
//! - Resource contention handling
//! - HTTP API endpoints

mod integration {
    pub mod api;
    pub mod recovery;
    pub mod resources;
    pub mod workflow;
}
