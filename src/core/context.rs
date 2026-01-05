//! Task execution context and inter-task communication.
//!
//! Tasks communicate via a shared key-value store. The architecture separates
//! reading (shared) from writing (buffered):
//!
//! - [`ContextStore`]: Shared read-only store of completed task outputs
//! - [`OutputBuffer`]: Task-local write buffer, merged on successful completion
//! - [`TaskContext`]: Combines store access, output buffer, and configuration

use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use thiserror::Error;

use super::types::TaskId;

/// Errors that can occur when working with the context.
#[derive(Debug, Error)]
pub enum ContextError {
    /// Key was not found in the context.
    #[error("key not found: {0}")]
    KeyNotFound(String),

    /// Failed to deserialize value from context.
    #[error("deserialization error for key '{key}': {message}")]
    DeserializationError { key: String, message: String },

    /// Failed to serialize value for context.
    #[error("serialization error for key '{key}': {message}")]
    SerializationError { key: String, message: String },

    /// Context lock was poisoned (concurrent access failure).
    #[error("context lock poisoned")]
    LockPoisoned,
}

/// Shared context store for a DAG execution.
///
/// This store is read-only for tasks. The executor merges task outputs
/// after successful completion via [`merge`](Self::merge).
///
/// # Thread Safety
///
/// `ContextStore` is `Clone` and can be shared across tasks. All read
/// operations acquire a read lock, allowing concurrent reads.
#[derive(Clone)]
pub struct ContextStore {
    inner: Arc<RwLock<HashMap<String, Value>>>,
}

impl ContextStore {
    /// Create a new empty context store.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get a value by key.
    ///
    /// Keys are typically namespaced as `"{task_id}.{key}"`.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Result<T, ContextError> {
        let store = self.inner.read().map_err(|_| ContextError::LockPoisoned)?;
        let value = store
            .get(key)
            .ok_or_else(|| ContextError::KeyNotFound(key.to_string()))?;
        serde_json::from_value(value.clone()).map_err(|e| ContextError::DeserializationError {
            key: key.to_string(),
            message: e.to_string(),
        })
    }

    /// Get an optional value by key. Returns None if key doesn't exist.
    pub fn get_optional<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let store = self.inner.read().ok()?;
        let value = store.get(key)?;
        serde_json::from_value(value.clone()).ok()
    }

    /// Check if a key exists in the context.
    pub fn contains(&self, key: &str) -> bool {
        self.inner
            .read()
            .map(|s| s.contains_key(key))
            .unwrap_or(false)
    }

    /// Get all keys in the context.
    pub fn keys(&self) -> Vec<String> {
        self.inner
            .read()
            .map(|s| s.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Merge a task's output buffer into the store.
    ///
    /// This should only be called by the executor after successful task completion.
    /// All outputs in the buffer are atomically added to the store.
    pub fn merge(&self, buffer: &OutputBuffer) -> Result<(), ContextError> {
        let mut store = self.inner.write().map_err(|_| ContextError::LockPoisoned)?;
        let outputs = buffer.outputs.borrow();
        for (key, value) in outputs.iter() {
            store.insert(key.clone(), value.clone());
        }
        Ok(())
    }
}

impl Default for ContextStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextStore {
    /// Create a store pre-populated with data (for testing).
    pub fn from_map(data: HashMap<String, Value>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(data)),
        }
    }
}

/// Task-local output buffer.
///
/// Tasks write outputs to this buffer during execution. The buffer is
/// merged into the shared [`ContextStore`] only after successful completion,
/// ensuring failed tasks don't pollute the store.
///
/// # Key Namespacing
///
/// All keys are automatically prefixed with the task ID. For example,
/// calling `set("result", 42)` from task "extract" creates key "extract.result".
pub struct OutputBuffer {
    task_id: TaskId,
    outputs: RefCell<HashMap<String, Value>>,
}

impl OutputBuffer {
    /// Create a new output buffer for a task.
    pub fn new(task_id: TaskId) -> Self {
        Self {
            task_id,
            outputs: RefCell::new(HashMap::new()),
        }
    }

    /// Write a value to the buffer.
    ///
    /// The key will be automatically prefixed with the task ID.
    /// e.g., setting "result" from task "extract" creates "extract.result"
    pub fn set<T: Serialize>(&self, key: &str, value: T) -> Result<(), ContextError> {
        let full_key = format!("{}.{}", self.task_id, key);
        let json_value =
            serde_json::to_value(value).map_err(|e| ContextError::SerializationError {
                key: full_key.clone(),
                message: e.to_string(),
            })?;
        self.outputs.borrow_mut().insert(full_key, json_value);
        Ok(())
    }

    /// Get the task ID associated with this buffer.
    pub fn task_id(&self) -> &TaskId {
        &self.task_id
    }

    /// Get the number of outputs in the buffer.
    pub fn len(&self) -> usize {
        self.outputs.borrow().len()
    }

    /// Check if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.outputs.borrow().is_empty()
    }

    /// Get all keys in the buffer.
    pub fn keys(&self) -> Vec<String> {
        self.outputs.borrow().keys().cloned().collect()
    }

    /// Get a value from the buffer (for testing/debugging).
    pub fn get_raw(&self, key: &str) -> Option<Value> {
        self.outputs.borrow().get(key).cloned()
    }
}

/// Execution context passed to tasks.
///
/// Provides access to:
/// - Inputs from upstream tasks (via shared [`ContextStore`])
/// - Output buffer for downstream tasks (via [`OutputBuffer`])
/// - Job-level configuration
pub struct TaskContext {
    /// Read inputs from upstream tasks (shared, read-only).
    pub inputs: ContextStore,

    /// Write outputs for downstream tasks (local buffer, merged on success).
    pub outputs: OutputBuffer,

    /// Job-level configuration values.
    pub config: Arc<HashMap<String, Value>>,
}

impl TaskContext {
    /// Create a new task context.
    ///
    /// # Arguments
    /// * `store` - Shared store for reading upstream task outputs
    /// * `task_id` - ID of the current task (for namespacing outputs)
    /// * `config` - Job-level configuration
    pub fn new(store: ContextStore, task_id: TaskId, config: Arc<HashMap<String, Value>>) -> Self {
        Self {
            inputs: store,
            outputs: OutputBuffer::new(task_id),
            config,
        }
    }

    /// Get a configuration value by key.
    pub fn get_config<T: DeserializeOwned>(&self, key: &str) -> Result<T, ContextError> {
        self.config
            .get(key)
            .ok_or_else(|| ContextError::KeyNotFound(key.to_string()))
            .and_then(|v| {
                serde_json::from_value(v.clone()).map_err(|e| ContextError::DeserializationError {
                    key: key.to_string(),
                    message: e.to_string(),
                })
            })
    }

    /// Get an optional configuration value.
    pub fn get_config_optional<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.config
            .get(key)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_store_get() {
        let store = ContextStore::new();
        // Manually insert for testing
        {
            let mut s = store.inner.write().unwrap();
            s.insert("my_key".to_string(), serde_json::json!(42));
        }

        let value: i32 = store.get("my_key").unwrap();
        assert_eq!(value, 42);
    }

    #[test]
    fn test_context_store_get_optional() {
        let store = ContextStore::new();
        {
            let mut s = store.inner.write().unwrap();
            s.insert("exists".to_string(), serde_json::json!(42));
        }

        let value: Option<i32> = store.get_optional("exists");
        assert_eq!(value, Some(42));

        let missing: Option<i32> = store.get_optional("missing");
        assert!(missing.is_none());
    }

    #[test]
    fn test_context_store_contains() {
        let store = ContextStore::new();
        {
            let mut s = store.inner.write().unwrap();
            s.insert("exists".to_string(), serde_json::json!(1));
        }

        assert!(store.contains("exists"));
        assert!(!store.contains("missing"));
    }

    #[test]
    fn test_context_store_keys() {
        let store = ContextStore::new();
        {
            let mut s = store.inner.write().unwrap();
            s.insert("a".to_string(), serde_json::json!(1));
            s.insert("b".to_string(), serde_json::json!(2));
        }

        let mut keys = store.keys();
        keys.sort();
        assert_eq!(keys, vec!["a", "b"]);
    }

    #[test]
    fn test_output_buffer_prefixes_keys() {
        let buffer = OutputBuffer::new(TaskId::new("extract"));
        buffer.set("row_count", 100).unwrap();

        let keys = buffer.keys();
        assert_eq!(keys, vec!["extract.row_count"]);
    }

    #[test]
    fn test_output_buffer_stores_values() {
        let buffer = OutputBuffer::new(TaskId::new("task1"));
        buffer.set("result", "hello").unwrap();

        let value = buffer.get_raw("task1.result").unwrap();
        assert_eq!(value, serde_json::json!("hello"));
    }

    #[test]
    fn test_context_store_merge() {
        let store = ContextStore::new();
        let buffer = OutputBuffer::new(TaskId::new("extract"));

        buffer.set("row_count", 100).unwrap();
        buffer.set("status", "success").unwrap();

        store.merge(&buffer).unwrap();

        let row_count: i32 = store.get("extract.row_count").unwrap();
        let status: String = store.get("extract.status").unwrap();

        assert_eq!(row_count, 100);
        assert_eq!(status, "success");
    }

    #[test]
    fn test_upstream_to_downstream_flow() {
        // Simulate: extract -> transform data flow
        let store = ContextStore::new();

        // Extract task writes to buffer
        let extract_buffer = OutputBuffer::new(TaskId::new("extract"));
        extract_buffer.set("rows", vec![1, 2, 3]).unwrap();

        // Merge extract's outputs to store
        store.merge(&extract_buffer).unwrap();

        // Transform task reads from store
        let rows: Vec<i32> = store.get("extract.rows").unwrap();
        assert_eq!(rows, vec![1, 2, 3]);
    }

    #[test]
    fn test_failed_task_outputs_not_merged() {
        let store = ContextStore::new();

        // Task writes some outputs
        let buffer = OutputBuffer::new(TaskId::new("failing_task"));
        buffer.set("partial", "data").unwrap();

        // Simulate failure - don't merge
        // (In real code, executor only calls merge on success)

        // Store should be empty
        assert!(!store.contains("failing_task.partial"));
    }

    #[test]
    fn test_task_context_creation() {
        let store = ContextStore::new();
        let mut config = HashMap::new();
        config.insert("batch_size".to_string(), serde_json::json!(100));

        let ctx = TaskContext::new(store, TaskId::new("my_task"), Arc::new(config));

        let batch_size: i32 = ctx.get_config("batch_size").unwrap();
        assert_eq!(batch_size, 100);
    }

    #[test]
    fn test_task_context_config_optional() {
        let store = ContextStore::new();
        let config = HashMap::new();

        let ctx = TaskContext::new(store, TaskId::new("task"), Arc::new(config));

        let missing: Option<i32> = ctx.get_config_optional("missing");
        assert!(missing.is_none());
    }

    #[test]
    fn test_context_store_clone_shares_data() {
        let store1 = ContextStore::new();
        let store2 = store1.clone();

        // Insert via store1
        {
            let mut s = store1.inner.write().unwrap();
            s.insert("key".to_string(), serde_json::json!("value"));
        }

        // Read via store2
        let value: String = store2.get("key").unwrap();
        assert_eq!(value, "value");
    }

    #[test]
    fn test_complex_type_serialization() {
        #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
        struct Record {
            id: u32,
            name: String,
        }

        let store = ContextStore::new();
        let buffer = OutputBuffer::new(TaskId::new("task"));

        let record = Record {
            id: 1,
            name: "test".to_string(),
        };
        buffer.set("record", &record).unwrap();
        store.merge(&buffer).unwrap();

        let retrieved: Record = store.get("task.record").unwrap();
        assert_eq!(retrieved, record);
    }
}
