//! Directed Acyclic Graph (DAG) for task dependencies.
//!
//! A DAG defines the execution order of tasks based on their dependencies.
//! Tasks can only run after all their dependencies have completed.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::task::Task;
use super::types::{DagId, TaskId};

/// Errors that can occur when working with DAGs.
#[derive(Debug, Error)]
pub enum DagError {
    /// A cycle was detected in the graph.
    #[error("cycle detected involving task: {0}")]
    CycleDetected(TaskId),

    /// A dependency references a task that doesn't exist.
    #[error("missing dependency: task '{from}' depends on non-existent task '{to}'")]
    MissingDependency { from: TaskId, to: TaskId },

    /// Attempted to add a duplicate task.
    #[error("duplicate task: {0}")]
    DuplicateTask(TaskId),

    /// Task not found in the DAG.
    #[error("task not found: {0}")]
    TaskNotFound(TaskId),
}

/// Conditions under which a task should execute based on upstream status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TaskCondition {
    /// Always run if dependencies are met (default).
    #[default]
    Always,

    /// Run only if all upstream tasks succeeded.
    AllSuccess,

    /// Run only if at least one upstream task failed.
    OnFailure,

    /// Run regardless of upstream task status (after they complete).
    AllDone,
}

/// A node in the DAG representing a task.
#[derive(Clone)]
pub struct TaskNode {
    /// The task ID.
    pub id: TaskId,

    /// The task implementation.
    pub task: Arc<dyn Task>,

    /// Condition for execution.
    pub condition: TaskCondition,
}

/// A Directed Acyclic Graph of tasks.
#[derive(Clone)]
pub struct Dag {
    /// Unique identifier for this DAG.
    id: DagId,

    /// Human-readable name.
    name: String,

    /// Task nodes indexed by ID.
    nodes: HashMap<TaskId, TaskNode>,

    /// Edges: task_id -> list of tasks it depends on.
    dependencies: HashMap<TaskId, Vec<TaskId>>,
}

impl Dag {
    /// Create a new empty DAG.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: DagId::new(id),
            name: name.into(),
            nodes: HashMap::new(),
            dependencies: HashMap::new(),
        }
    }

    /// Get the DAG ID.
    pub fn id(&self) -> &DagId {
        &self.id
    }

    /// Get the DAG name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Check if the DAG is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Get the number of tasks in the DAG.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Add a task to the DAG with no dependencies.
    pub fn add_task(&mut self, task: Arc<dyn Task>) -> Result<(), DagError> {
        self.add_task_with_condition(task, TaskCondition::default())
    }

    /// Add a task with a specific execution condition.
    pub fn add_task_with_condition(
        &mut self,
        task: Arc<dyn Task>,
        condition: TaskCondition,
    ) -> Result<(), DagError> {
        let id = TaskId::new(task.name());
        if self.nodes.contains_key(&id) {
            return Err(DagError::DuplicateTask(id));
        }

        self.nodes.insert(
            id.clone(),
            TaskNode {
                id: id.clone(),
                task,
                condition,
            },
        );
        self.dependencies.insert(id, Vec::new());
        Ok(())
    }

    /// Add a dependency: `from` depends on `to` (to must complete before from).
    pub fn add_dependency(&mut self, from: &TaskId, to: &TaskId) -> Result<(), DagError> {
        // Check both tasks exist
        if !self.nodes.contains_key(from) {
            return Err(DagError::TaskNotFound(from.clone()));
        }
        if !self.nodes.contains_key(to) {
            return Err(DagError::MissingDependency {
                from: from.clone(),
                to: to.clone(),
            });
        }

        // Add the dependency
        self.dependencies
            .entry(from.clone())
            .or_default()
            .push(to.clone());

        Ok(())
    }

    /// Get a task node by ID.
    pub fn get_task(&self, id: &TaskId) -> Option<&TaskNode> {
        self.nodes.get(id)
    }

    /// Get the dependencies of a task.
    pub fn get_dependencies(&self, id: &TaskId) -> Option<&[TaskId]> {
        self.dependencies.get(id).map(|v| v.as_slice())
    }

    /// Get tasks that depend on the given task (downstream tasks).
    pub fn get_downstream(&self, id: &TaskId) -> Vec<TaskId> {
        self.dependencies
            .iter()
            .filter_map(|(task_id, deps)| {
                if deps.contains(id) {
                    Some(task_id.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get tasks that are ready to execute (no pending dependencies).
    pub fn get_ready_tasks(&self, completed: &HashSet<TaskId>) -> Vec<TaskId> {
        self.nodes
            .keys()
            .filter(|id| {
                // Not already completed
                if completed.contains(*id) {
                    return false;
                }
                // All dependencies are completed
                self.dependencies
                    .get(*id)
                    .map(|deps| deps.iter().all(|dep| completed.contains(dep)))
                    .unwrap_or(true)
            })
            .cloned()
            .collect()
    }

    /// Validate the DAG and return tasks in topological order.
    ///
    /// Returns an error if cycles are detected or dependencies are missing.
    pub fn topological_sort(&self) -> Result<Vec<TaskId>, DagError> {
        // Kahn's algorithm
        let mut in_degree: HashMap<TaskId, usize> = HashMap::new();
        let mut reverse_deps: HashMap<TaskId, Vec<TaskId>> = HashMap::new();

        // Initialize in-degrees
        for id in self.nodes.keys() {
            in_degree.insert(id.clone(), 0);
            reverse_deps.insert(id.clone(), Vec::new());
        }

        // Calculate in-degrees and reverse dependencies
        for (from, deps) in &self.dependencies {
            in_degree.insert(from.clone(), deps.len());
            for to in deps {
                reverse_deps
                    .entry(to.clone())
                    .or_default()
                    .push(from.clone());
            }
        }

        // Start with nodes that have no dependencies
        let mut queue: VecDeque<TaskId> = in_degree
            .iter()
            .filter(|(_, degree)| **degree == 0)
            .map(|(id, _)| id.clone())
            .collect();

        let mut result = Vec::new();

        while let Some(id) = queue.pop_front() {
            result.push(id.clone());

            // Reduce in-degree for all downstream tasks
            if let Some(downstream) = reverse_deps.get(&id) {
                for next in downstream {
                    if let Some(degree) = in_degree.get_mut(next) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push_back(next.clone());
                        }
                    }
                }
            }
        }

        // If we didn't visit all nodes, there's a cycle
        if result.len() != self.nodes.len() {
            // Find a node that's part of the cycle
            let cycle_node = in_degree
                .iter()
                .find(|(_, degree)| **degree > 0)
                .map(|(id, _)| id.clone())
                .unwrap();
            return Err(DagError::CycleDetected(cycle_node));
        }

        Ok(result)
    }

    /// Validate the DAG structure.
    pub fn validate(&self) -> Result<(), DagError> {
        // Check for missing dependencies
        for (from, deps) in &self.dependencies {
            for to in deps {
                if !self.nodes.contains_key(to) {
                    return Err(DagError::MissingDependency {
                        from: from.clone(),
                        to: to.clone(),
                    });
                }
            }
        }

        // Check for cycles
        self.topological_sort()?;

        Ok(())
    }

    /// Get all task IDs in the DAG.
    pub fn task_ids(&self) -> Vec<TaskId> {
        self.nodes.keys().cloned().collect()
    }
}

/// Builder for constructing DAGs fluently.
pub struct DagBuilder {
    dag: Dag,
}

impl DagBuilder {
    /// Create a new DAG builder.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            dag: Dag::new(id, name),
        }
    }

    /// Add a task to the DAG.
    pub fn add_task(mut self, task: Arc<dyn Task>) -> Self {
        let _ = self.dag.add_task(task);
        self
    }

    /// Add a task with dependencies.
    pub fn add_task_with_deps(mut self, task: Arc<dyn Task>, depends_on: &[&str]) -> Self {
        let task_id = TaskId::new(task.name());
        let _ = self.dag.add_task(task);
        for dep in depends_on {
            let _ = self.dag.add_dependency(&task_id, &TaskId::new(*dep));
        }
        self
    }

    /// Add a task with dependencies and a condition.
    pub fn add_task_with_deps_and_condition(
        mut self,
        task: Arc<dyn Task>,
        depends_on: &[&str],
        condition: TaskCondition,
    ) -> Self {
        let task_id = TaskId::new(task.name());
        let _ = self.dag.add_task_with_condition(task, condition);
        for dep in depends_on {
            let _ = self.dag.add_dependency(&task_id, &TaskId::new(*dep));
        }
        self
    }

    /// Add a dependency between tasks.
    pub fn add_dependency(mut self, from: &str, to: &str) -> Self {
        let _ = self
            .dag
            .add_dependency(&TaskId::new(from), &TaskId::new(to));
        self
    }

    /// Build the DAG, validating it in the process.
    pub fn build(self) -> Result<Dag, DagError> {
        self.dag.validate()?;
        Ok(self.dag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::TaskContext;
    use crate::core::task::TaskError;
    use async_trait::async_trait;

    // Simple test task implementation
    struct TestTask {
        name: String,
    }

    impl TestTask {
        fn new(name: &str) -> Arc<dyn Task> {
            Arc::new(Self {
                name: name.to_string(),
            })
        }
    }

    #[async_trait]
    impl Task for TestTask {
        fn name(&self) -> &str {
            &self.name
        }

        async fn execute(&self, _ctx: &mut TaskContext) -> Result<(), TaskError> {
            Ok(())
        }
    }

    #[test]
    fn test_create_empty_dag() {
        let dag = Dag::new("test_dag", "Test DAG");

        assert_eq!(dag.id().as_str(), "test_dag");
        assert_eq!(dag.name(), "Test DAG");
        assert!(dag.is_empty());
        assert_eq!(dag.len(), 0);
    }

    #[test]
    fn test_add_single_task() {
        let mut dag = Dag::new("dag", "DAG");
        let task = TestTask::new("task_a");

        dag.add_task(task).unwrap();

        assert_eq!(dag.len(), 1);
        assert!(dag.get_task(&TaskId::new("task_a")).is_some());
    }

    #[test]
    fn test_add_task_with_dependency() {
        let mut dag = Dag::new("dag", "DAG");

        dag.add_task(TestTask::new("task_a")).unwrap();
        dag.add_task(TestTask::new("task_b")).unwrap();
        dag.add_dependency(&TaskId::new("task_b"), &TaskId::new("task_a"))
            .unwrap();

        let deps = dag.get_dependencies(&TaskId::new("task_b")).unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].as_str(), "task_a");
    }

    #[test]
    fn test_topological_order() {
        // A -> B -> C (linear chain)
        let mut dag = Dag::new("dag", "DAG");

        dag.add_task(TestTask::new("A")).unwrap();
        dag.add_task(TestTask::new("B")).unwrap();
        dag.add_task(TestTask::new("C")).unwrap();

        dag.add_dependency(&TaskId::new("B"), &TaskId::new("A"))
            .unwrap();
        dag.add_dependency(&TaskId::new("C"), &TaskId::new("B"))
            .unwrap();

        let order = dag.topological_sort().unwrap();

        // A must come before B, B must come before C
        let pos_a = order.iter().position(|id| id.as_str() == "A").unwrap();
        let pos_b = order.iter().position(|id| id.as_str() == "B").unwrap();
        let pos_c = order.iter().position(|id| id.as_str() == "C").unwrap();

        assert!(pos_a < pos_b);
        assert!(pos_b < pos_c);
    }

    #[test]
    fn test_detect_cycle() {
        let mut dag = Dag::new("dag", "DAG");

        dag.add_task(TestTask::new("A")).unwrap();
        dag.add_task(TestTask::new("B")).unwrap();
        dag.add_task(TestTask::new("C")).unwrap();

        // Create cycle: A -> B -> C -> A
        dag.add_dependency(&TaskId::new("B"), &TaskId::new("A"))
            .unwrap();
        dag.add_dependency(&TaskId::new("C"), &TaskId::new("B"))
            .unwrap();
        dag.add_dependency(&TaskId::new("A"), &TaskId::new("C"))
            .unwrap();

        let result = dag.topological_sort();
        assert!(matches!(result, Err(DagError::CycleDetected(_))));
    }

    #[test]
    fn test_detect_missing_dependency() {
        let mut dag = Dag::new("dag", "DAG");

        dag.add_task(TestTask::new("A")).unwrap();

        let result = dag.add_dependency(&TaskId::new("A"), &TaskId::new("nonexistent"));
        assert!(matches!(result, Err(DagError::MissingDependency { .. })));
    }

    #[test]
    fn test_task_conditions() {
        let mut dag = Dag::new("dag", "DAG");

        dag.add_task_with_condition(TestTask::new("always"), TaskCondition::Always)
            .unwrap();
        dag.add_task_with_condition(TestTask::new("on_success"), TaskCondition::AllSuccess)
            .unwrap();
        dag.add_task_with_condition(TestTask::new("on_failure"), TaskCondition::OnFailure)
            .unwrap();
        dag.add_task_with_condition(TestTask::new("all_done"), TaskCondition::AllDone)
            .unwrap();

        assert_eq!(
            dag.get_task(&TaskId::new("always")).unwrap().condition,
            TaskCondition::Always
        );
        assert_eq!(
            dag.get_task(&TaskId::new("on_success")).unwrap().condition,
            TaskCondition::AllSuccess
        );
        assert_eq!(
            dag.get_task(&TaskId::new("on_failure")).unwrap().condition,
            TaskCondition::OnFailure
        );
        assert_eq!(
            dag.get_task(&TaskId::new("all_done")).unwrap().condition,
            TaskCondition::AllDone
        );
    }

    #[test]
    fn test_get_ready_tasks() {
        // Diamond: A -> B, C -> D
        let mut dag = Dag::new("dag", "DAG");

        dag.add_task(TestTask::new("A")).unwrap();
        dag.add_task(TestTask::new("B")).unwrap();
        dag.add_task(TestTask::new("C")).unwrap();
        dag.add_task(TestTask::new("D")).unwrap();

        dag.add_dependency(&TaskId::new("B"), &TaskId::new("A"))
            .unwrap();
        dag.add_dependency(&TaskId::new("C"), &TaskId::new("A"))
            .unwrap();
        dag.add_dependency(&TaskId::new("D"), &TaskId::new("B"))
            .unwrap();
        dag.add_dependency(&TaskId::new("D"), &TaskId::new("C"))
            .unwrap();

        // Initially, only A is ready
        let ready = dag.get_ready_tasks(&HashSet::new());
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].as_str(), "A");

        // After A completes, B and C are ready
        let mut completed = HashSet::new();
        completed.insert(TaskId::new("A"));
        let mut ready = dag.get_ready_tasks(&completed);
        ready.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        assert_eq!(ready.len(), 2);
        assert_eq!(ready[0].as_str(), "B");
        assert_eq!(ready[1].as_str(), "C");

        // After B and C complete, D is ready
        completed.insert(TaskId::new("B"));
        completed.insert(TaskId::new("C"));
        let ready = dag.get_ready_tasks(&completed);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].as_str(), "D");
    }

    #[test]
    fn test_get_downstream_tasks() {
        let mut dag = Dag::new("dag", "DAG");

        dag.add_task(TestTask::new("A")).unwrap();
        dag.add_task(TestTask::new("B")).unwrap();
        dag.add_task(TestTask::new("C")).unwrap();

        dag.add_dependency(&TaskId::new("B"), &TaskId::new("A"))
            .unwrap();
        dag.add_dependency(&TaskId::new("C"), &TaskId::new("A"))
            .unwrap();

        let mut downstream = dag.get_downstream(&TaskId::new("A"));
        downstream.sort_by(|a, b| a.as_str().cmp(b.as_str()));

        assert_eq!(downstream.len(), 2);
        assert_eq!(downstream[0].as_str(), "B");
        assert_eq!(downstream[1].as_str(), "C");
    }

    #[test]
    fn test_dag_builder() {
        let dag = DagBuilder::new("pipeline", "Data Pipeline")
            .add_task(TestTask::new("extract"))
            .add_task_with_deps(TestTask::new("transform"), &["extract"])
            .add_task_with_deps(TestTask::new("load"), &["transform"])
            .build()
            .unwrap();

        assert_eq!(dag.len(), 3);

        let order = dag.topological_sort().unwrap();
        let names: Vec<&str> = order.iter().map(|id| id.as_str()).collect();
        assert_eq!(names, vec!["extract", "transform", "load"]);
    }

    #[test]
    fn test_duplicate_task_error() {
        let mut dag = Dag::new("dag", "DAG");

        dag.add_task(TestTask::new("A")).unwrap();
        let result = dag.add_task(TestTask::new("A"));

        assert!(matches!(result, Err(DagError::DuplicateTask(_))));
    }

    #[test]
    fn test_validate_dag() {
        let mut dag = Dag::new("dag", "DAG");

        dag.add_task(TestTask::new("A")).unwrap();
        dag.add_task(TestTask::new("B")).unwrap();
        dag.add_dependency(&TaskId::new("B"), &TaskId::new("A"))
            .unwrap();

        assert!(dag.validate().is_ok());
    }
}
