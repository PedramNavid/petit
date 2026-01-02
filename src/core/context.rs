//! Task execution context and inter-task communication.
//!
//! Tasks communicate via a shared key-value store. Upstream tasks write
//! outputs that downstream tasks can read as inputs.

use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
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

/// Execution context passed to tasks.
///
/// Provides access to:
/// - Inputs from upstream tasks
/// - Output writer for downstream tasks
/// - Job-level configuration
/// - Execution metadata
pub struct TaskContext {
    /// Read inputs from upstream tasks.
    pub inputs: ContextReader,

    /// Write outputs for downstream tasks.
    pub outputs: ContextWriter,

    /// Job-level configuration values.
    pub config: Arc<HashMap<String, Value>>,
}

/// Reader for accessing upstream task outputs.
#[derive(Clone)]
pub struct ContextReader {
    store: Arc<RwLock<HashMap<String, Value>>>,
}

/// Writer for storing task outputs.
pub struct ContextWriter {
    store: Arc<RwLock<HashMap<String, Value>>>,
    task_id: TaskId,
}

impl TaskContext {
    /// Create a new task context.
    ///
    /// # Arguments
    /// * `store` - Shared store for inter-task communication
    /// * `task_id` - ID of the current task (for namespacing outputs)
    /// * `config` - Job-level configuration
    pub fn new(
        store: Arc<RwLock<HashMap<String, Value>>>,
        task_id: TaskId,
        config: Arc<HashMap<String, Value>>,
    ) -> Self {
        Self {
            inputs: ContextReader::new(store.clone()),
            outputs: ContextWriter::new(store, task_id),
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

impl ContextReader {
    /// Create a new context reader.
    pub fn new(store: Arc<RwLock<HashMap<String, Value>>>) -> Self {
        Self { store }
    }

    /// Get a value by key.
    ///
    /// Keys can be simple ("my_key") or namespaced ("task_id.my_key").
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Result<T, ContextError> {
        let store = self.store.read().map_err(|_| ContextError::LockPoisoned)?;
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
        let store = self.store.read().ok()?;
        let value = store.get(key)?;
        serde_json::from_value(value.clone()).ok()
    }

    /// Check if a key exists in the context.
    pub fn contains(&self, key: &str) -> bool {
        self.store
            .read()
            .map(|s| s.contains_key(key))
            .unwrap_or(false)
    }

    /// Get all keys in the context.
    pub fn keys(&self) -> Vec<String> {
        self.store
            .read()
            .map(|s| s.keys().cloned().collect())
            .unwrap_or_default()
    }
}

impl ContextWriter {
    /// Create a new context writer for a specific task.
    pub fn new(store: Arc<RwLock<HashMap<String, Value>>>, task_id: TaskId) -> Self {
        Self { store, task_id }
    }

    /// Set a value in the context.
    ///
    /// The key will be automatically prefixed with the task ID.
    /// e.g., setting "result" from task "extract" creates "extract.result"
    pub fn set<T: Serialize>(&self, key: &str, value: T) -> Result<(), ContextError> {
        let full_key = format!("{}.{}", self.task_id, key);
        self.set_raw(&full_key, value)
    }

    /// Set a value with an explicit full key (no auto-prefixing).
    pub fn set_raw<T: Serialize>(&self, key: &str, value: T) -> Result<(), ContextError> {
        let json_value =
            serde_json::to_value(value).map_err(|e| ContextError::SerializationError {
                key: key.to_string(),
                message: e.to_string(),
            })?;
        let mut store = self.store.write().map_err(|_| ContextError::LockPoisoned)?;
        store.insert(key.to_string(), json_value);
        Ok(())
    }

    /// Get the task ID associated with this writer.
    pub fn task_id(&self) -> &TaskId {
        &self.task_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_empty_store() -> Arc<RwLock<HashMap<String, Value>>> {
        Arc::new(RwLock::new(HashMap::new()))
    }

    #[test]
    fn test_context_writer_prefixes_keys() {
        let store = create_empty_store();
        let writer = ContextWriter::new(store.clone(), TaskId::new("extract"));

        writer.set("row_count", 100).unwrap();

        let reader = ContextReader::new(store);
        let value: i32 = reader.get("extract.row_count").unwrap();
        assert_eq!(value, 100);
    }

    #[test]
    fn test_context_reader_get_typed_value() {
        let store = create_empty_store();
        {
            let mut s = store.write().unwrap();
            s.insert("my_key".to_string(), serde_json::json!(42));
        }

        let reader = ContextReader::new(store);
        let value: i32 = reader.get("my_key").unwrap();
        assert_eq!(value, 42);
    }

    #[test]
    fn test_context_reader_get_string() {
        let store = create_empty_store();
        {
            let mut s = store.write().unwrap();
            s.insert("name".to_string(), serde_json::json!("hello"));
        }

        let reader = ContextReader::new(store);
        let value: String = reader.get("name").unwrap();
        assert_eq!(value, "hello");
    }

    #[test]
    fn test_context_reader_get_complex_type() {
        #[derive(Debug, PartialEq, serde::Deserialize)]
        struct Record {
            id: u32,
            name: String,
        }

        let store = create_empty_store();
        {
            let mut s = store.write().unwrap();
            s.insert(
                "record".to_string(),
                serde_json::json!({"id": 1, "name": "test"}),
            );
        }

        let reader = ContextReader::new(store);
        let value: Record = reader.get("record").unwrap();
        assert_eq!(
            value,
            Record {
                id: 1,
                name: "test".to_string()
            }
        );
    }

    #[test]
    fn test_context_reader_key_not_found() {
        let store = create_empty_store();
        let reader = ContextReader::new(store);

        let result: Result<i32, _> = reader.get("nonexistent");
        assert!(matches!(result, Err(ContextError::KeyNotFound(_))));
    }

    #[test]
    fn test_context_reader_optional_returns_none() {
        let store = create_empty_store();
        let reader = ContextReader::new(store);

        let value: Option<i32> = reader.get_optional("nonexistent");
        assert!(value.is_none());
    }

    #[test]
    fn test_context_reader_optional_returns_some() {
        let store = create_empty_store();
        {
            let mut s = store.write().unwrap();
            s.insert("exists".to_string(), serde_json::json!(42));
        }

        let reader = ContextReader::new(store);
        let value: Option<i32> = reader.get_optional("exists");
        assert_eq!(value, Some(42));
    }

    #[test]
    fn test_context_reader_contains() {
        let store = create_empty_store();
        {
            let mut s = store.write().unwrap();
            s.insert("exists".to_string(), serde_json::json!(1));
        }

        let reader = ContextReader::new(store);
        assert!(reader.contains("exists"));
        assert!(!reader.contains("missing"));
    }

    #[test]
    fn test_context_reader_keys() {
        let store = create_empty_store();
        {
            let mut s = store.write().unwrap();
            s.insert("a".to_string(), serde_json::json!(1));
            s.insert("b".to_string(), serde_json::json!(2));
        }

        let reader = ContextReader::new(store);
        let mut keys = reader.keys();
        keys.sort();
        assert_eq!(keys, vec!["a", "b"]);
    }

    #[test]
    fn test_context_writer_set_raw_no_prefix() {
        let store = create_empty_store();
        let writer = ContextWriter::new(store.clone(), TaskId::new("task1"));

        writer.set_raw("global_key", "value").unwrap();

        let reader = ContextReader::new(store);
        let value: String = reader.get("global_key").unwrap();
        assert_eq!(value, "value");
    }

    #[test]
    fn test_task_context_creation() {
        let store = create_empty_store();
        let mut config = HashMap::new();
        config.insert("batch_size".to_string(), serde_json::json!(100));

        let ctx = TaskContext::new(store, TaskId::new("my_task"), Arc::new(config));

        let batch_size: i32 = ctx.get_config("batch_size").unwrap();
        assert_eq!(batch_size, 100);
    }

    #[test]
    fn test_task_context_config_optional() {
        let store = create_empty_store();
        let config = HashMap::new();

        let ctx = TaskContext::new(store, TaskId::new("task"), Arc::new(config));

        let missing: Option<i32> = ctx.get_config_optional("missing");
        assert!(missing.is_none());
    }

    #[test]
    fn test_upstream_to_downstream_flow() {
        // Simulate: extract -> transform data flow
        let store = create_empty_store();

        // Extract task writes output
        let extract_writer = ContextWriter::new(store.clone(), TaskId::new("extract"));
        extract_writer.set("rows", vec![1, 2, 3]).unwrap();

        // Transform task reads input
        let transform_reader = ContextReader::new(store);
        let rows: Vec<i32> = transform_reader.get("extract.rows").unwrap();
        assert_eq!(rows, vec![1, 2, 3]);
    }
}
