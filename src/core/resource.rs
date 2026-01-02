//! Resource requirements for task execution.
//!
//! Tasks can declare both abstract resource slots (named pools) and
//! system resource constraints (CPU, memory).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Resource requirements for a task.
///
/// Combines abstract named resource pools with system resource constraints.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ResourceRequirements {
    /// Abstract named resource pools (e.g., "gpu": 2, "db_conn": 1).
    /// The executor will acquire these from configured pools before running.
    pub slots: HashMap<String, u32>,

    /// System resource constraints.
    pub system: SystemResources,
}

/// System resource constraints for a task.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SystemResources {
    /// CPU cores (can be fractional, e.g., 0.5 for half a core).
    pub cpu_cores: Option<f32>,

    /// Memory limit in bytes.
    pub memory_bytes: Option<u64>,
}

impl ResourceRequirements {
    /// Create empty resource requirements (no constraints).
    pub fn none() -> Self {
        Self::default()
    }

    /// Create requirements with only slot constraints.
    pub fn slots(slots: HashMap<String, u32>) -> Self {
        Self {
            slots,
            system: SystemResources::default(),
        }
    }

    /// Create requirements with only system constraints.
    pub fn system(system: SystemResources) -> Self {
        Self {
            slots: HashMap::new(),
            system,
        }
    }

    /// Builder: add a slot requirement.
    pub fn with_slot(mut self, name: impl Into<String>, count: u32) -> Self {
        self.slots.insert(name.into(), count);
        self
    }

    /// Builder: set CPU cores requirement.
    pub fn with_cpu(mut self, cores: f32) -> Self {
        self.system.cpu_cores = Some(cores);
        self
    }

    /// Builder: set memory requirement in bytes.
    pub fn with_memory(mut self, bytes: u64) -> Self {
        self.system.memory_bytes = Some(bytes);
        self
    }

    /// Check if this has any requirements.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
            && self.system.cpu_cores.is_none()
            && self.system.memory_bytes.is_none()
    }
}

impl SystemResources {
    /// Create empty system resources (no constraints).
    pub fn none() -> Self {
        Self::default()
    }

    /// Create with CPU constraint only.
    pub fn cpu(cores: f32) -> Self {
        Self {
            cpu_cores: Some(cores),
            memory_bytes: None,
        }
    }

    /// Create with memory constraint only.
    pub fn memory(bytes: u64) -> Self {
        Self {
            cpu_cores: None,
            memory_bytes: Some(bytes),
        }
    }

    /// Create with both CPU and memory constraints.
    pub fn new(cpu_cores: Option<f32>, memory_bytes: Option<u64>) -> Self {
        Self {
            cpu_cores,
            memory_bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_requirements_are_empty() {
        let req = ResourceRequirements::default();

        assert!(req.slots.is_empty());
        assert!(req.system.cpu_cores.is_none());
        assert!(req.system.memory_bytes.is_none());
        assert!(req.is_empty());
    }

    #[test]
    fn test_requirements_with_slots() {
        let req = ResourceRequirements::none()
            .with_slot("gpu", 2)
            .with_slot("db_conn", 1);

        assert_eq!(req.slots.get("gpu"), Some(&2));
        assert_eq!(req.slots.get("db_conn"), Some(&1));
        assert!(!req.is_empty());
    }

    #[test]
    fn test_requirements_with_system_resources() {
        let req = ResourceRequirements::none()
            .with_cpu(2.5)
            .with_memory(1024 * 1024 * 512); // 512 MB

        assert_eq!(req.system.cpu_cores, Some(2.5));
        assert_eq!(req.system.memory_bytes, Some(536_870_912));
    }

    #[test]
    fn test_combined_requirements() {
        let req = ResourceRequirements::none()
            .with_slot("gpu", 1)
            .with_cpu(4.0)
            .with_memory(1024 * 1024 * 1024); // 1 GB

        assert_eq!(req.slots.get("gpu"), Some(&1));
        assert_eq!(req.system.cpu_cores, Some(4.0));
        assert_eq!(req.system.memory_bytes, Some(1_073_741_824));
    }

    #[test]
    fn test_system_resources_constructors() {
        let cpu_only = SystemResources::cpu(2.0);
        assert_eq!(cpu_only.cpu_cores, Some(2.0));
        assert!(cpu_only.memory_bytes.is_none());

        let mem_only = SystemResources::memory(1024);
        assert!(mem_only.cpu_cores.is_none());
        assert_eq!(mem_only.memory_bytes, Some(1024));

        let both = SystemResources::new(Some(1.5), Some(2048));
        assert_eq!(both.cpu_cores, Some(1.5));
        assert_eq!(both.memory_bytes, Some(2048));
    }

    #[test]
    fn test_slots_constructor() {
        let mut slots = HashMap::new();
        slots.insert("gpu".to_string(), 2);
        slots.insert("network".to_string(), 1);

        let req = ResourceRequirements::slots(slots);

        assert_eq!(req.slots.len(), 2);
        assert!(req.system.cpu_cores.is_none());
    }
}
