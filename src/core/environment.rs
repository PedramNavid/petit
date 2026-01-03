//! Environment variables for task execution.
//!
//! Tasks receive environment variables that can contain credentials,
//! configuration, and other runtime values. Similar to Docker/Kubernetes
//! environment configuration.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Environment variables passed to a task during execution.
///
/// This provides a simple way to pass configuration, credentials,
/// and other runtime values to tasks without complex resource management.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Environment {
    /// Environment variables as key-value pairs.
    vars: HashMap<String, String>,
}

impl Environment {
    /// Create an empty environment.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an environment from a HashMap.
    pub fn from_map(vars: HashMap<String, String>) -> Self {
        Self { vars }
    }

    /// Builder: add an environment variable.
    pub fn with_var(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.vars.insert(key.into(), value.into());
        self
    }

    /// Add an environment variable.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.vars.insert(key.into(), value.into());
    }

    /// Get an environment variable.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(|s| s.as_str())
    }

    /// Check if a variable exists.
    pub fn contains(&self, key: &str) -> bool {
        self.vars.contains_key(key)
    }

    /// Get all variables as a reference to the internal map.
    pub fn vars(&self) -> &HashMap<String, String> {
        &self.vars
    }

    /// Get all variables as a mutable reference.
    pub fn vars_mut(&mut self) -> &mut HashMap<String, String> {
        &mut self.vars
    }

    /// Check if the environment is empty.
    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }

    /// Get the number of variables.
    pub fn len(&self) -> usize {
        self.vars.len()
    }

    /// Merge another environment into this one.
    /// Variables from `other` override existing variables.
    pub fn merge(&mut self, other: &Environment) {
        for (k, v) in &other.vars {
            self.vars.insert(k.clone(), v.clone());
        }
    }

    /// Create a new environment by merging this one with another.
    /// Variables from `other` override existing variables.
    pub fn merged_with(&self, other: &Environment) -> Self {
        let mut result = self.clone();
        result.merge(other);
        result
    }

    /// Convert to a Vec of (key, value) pairs for use with Command.
    pub fn to_vec(&self) -> Vec<(String, String)> {
        self.vars
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Iterate over the environment variables.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &String)> {
        self.vars.iter()
    }
}

impl FromIterator<(String, String)> for Environment {
    fn from_iter<I: IntoIterator<Item = (String, String)>>(iter: I) -> Self {
        Self {
            vars: iter.into_iter().collect(),
        }
    }
}

impl<'a> FromIterator<(&'a str, &'a str)> for Environment {
    fn from_iter<I: IntoIterator<Item = (&'a str, &'a str)>>(iter: I) -> Self {
        Self {
            vars: iter
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_environment() {
        let env = Environment::new();

        assert!(env.is_empty());
        assert_eq!(env.len(), 0);
    }

    #[test]
    fn test_environment_builder() {
        let env = Environment::new()
            .with_var("DATABASE_URL", "postgres://localhost/db")
            .with_var("API_KEY", "secret123");

        assert_eq!(env.len(), 2);
        assert_eq!(env.get("DATABASE_URL"), Some("postgres://localhost/db"));
        assert_eq!(env.get("API_KEY"), Some("secret123"));
    }

    #[test]
    fn test_environment_set_get() {
        let mut env = Environment::new();

        env.set("FOO", "bar");
        assert_eq!(env.get("FOO"), Some("bar"));
        assert!(env.contains("FOO"));
        assert!(!env.contains("BAZ"));
    }

    #[test]
    fn test_environment_from_map() {
        let mut map = HashMap::new();
        map.insert("KEY1".to_string(), "value1".to_string());
        map.insert("KEY2".to_string(), "value2".to_string());

        let env = Environment::from_map(map);

        assert_eq!(env.len(), 2);
        assert_eq!(env.get("KEY1"), Some("value1"));
    }

    #[test]
    fn test_environment_merge() {
        let mut base = Environment::new().with_var("A", "1").with_var("B", "2");

        let override_env = Environment::new()
            .with_var("B", "overridden")
            .with_var("C", "3");

        base.merge(&override_env);

        assert_eq!(base.get("A"), Some("1"));
        assert_eq!(base.get("B"), Some("overridden"));
        assert_eq!(base.get("C"), Some("3"));
    }

    #[test]
    fn test_environment_merged_with() {
        let base = Environment::new().with_var("A", "1").with_var("B", "2");

        let other = Environment::new().with_var("B", "overridden");

        let merged = base.merged_with(&other);

        // Original unchanged
        assert_eq!(base.get("B"), Some("2"));
        // Merged has override
        assert_eq!(merged.get("B"), Some("overridden"));
    }

    #[test]
    fn test_environment_to_vec() {
        let env = Environment::new().with_var("A", "1").with_var("B", "2");

        let vec = env.to_vec();
        assert_eq!(vec.len(), 2);
    }

    #[test]
    fn test_environment_from_iterator() {
        let env: Environment = vec![("KEY1", "value1"), ("KEY2", "value2")]
            .into_iter()
            .collect();

        assert_eq!(env.len(), 2);
        assert_eq!(env.get("KEY1"), Some("value1"));
    }

    #[test]
    fn test_environment_serialization() {
        let env = Environment::new()
            .with_var("DB", "postgres")
            .with_var("PORT", "5432");

        let json = serde_json::to_string(&env).unwrap();
        let deserialized: Environment = serde_json::from_str(&json).unwrap();

        assert_eq!(env, deserialized);
    }
}
