//! API Sketch: How users will define and run jobs via YAML
//!
//! This test demonstrates the intended user experience before we build
//! the full YAML parsing infrastructure. It helps validate our API design.

use petit::{DagBuilder, Environment, Task, TaskContext, TaskError, TaskId};
use async_trait::async_trait;
use std::sync::Arc;

/// This is what a user's YAML job definition would look like:
///
/// ```yaml
/// # jobs/etl_pipeline.yaml
/// name: etl_pipeline
/// schedule: "0 0 2 * * *"  # 2 AM daily
///
/// # Job-level environment (inherited by all tasks)
/// environment:
///   LOG_LEVEL: info
///   DATA_DIR: /data
///
/// # Job-level config (accessible via context)
/// config:
///   input_path: /data/raw
///   output_path: /data/processed
///
/// tasks:
///   extract:
///     type: command
///     program: python
///     args: ["scripts/extract.py", "--input", "${config.input_path}"]
///     environment:
///       DATABASE_URL: postgres://localhost/source
///       BATCH_SIZE: "1000"
///     retry:
///       max_attempts: 3
///       delay_seconds: 60
///
///   transform:
///     type: command
///     program: python
///     args: ["scripts/transform.py"]
///     depends_on: [extract]
///     environment:
///       SPARK_MASTER: local[4]
///
///   validate:
///     type: command
///     program: python
///     args: ["scripts/validate.py"]
///     depends_on: [transform]
///     condition: all_success
///
///   notify_success:
///     type: command
///     program: bash
///     args: ["-c", "echo 'Pipeline completed!'"]
///     depends_on: [validate]
///     condition: all_success
///     environment:
///       SLACK_WEBHOOK: https://hooks.slack.com/...
///
///   notify_failure:
///     type: command
///     program: bash
///     args: ["-c", "echo 'Pipeline failed!'"]
///     depends_on: [validate]
///     condition: on_failure
/// ```

// For now, we simulate what the YAML loader would create
// using our current Rust API

struct MockCommandTask {
    name: String,
    program: String,
    args: Vec<String>,
    env: Environment,
}

impl MockCommandTask {
    fn new(name: &str, program: &str, args: Vec<&str>) -> Arc<dyn Task> {
        Arc::new(Self {
            name: name.to_string(),
            program: program.to_string(),
            args: args.into_iter().map(String::from).collect(),
            env: Environment::new(),
        })
    }

    fn with_env(name: &str, program: &str, args: Vec<&str>, env: Environment) -> Arc<dyn Task> {
        Arc::new(Self {
            name: name.to_string(),
            program: program.to_string(),
            args: args.into_iter().map(String::from).collect(),
            env,
        })
    }
}

#[async_trait]
impl Task for MockCommandTask {
    fn name(&self) -> &str {
        &self.name
    }

    async fn execute(&self, ctx: &mut TaskContext) -> Result<(), TaskError> {
        // In real implementation, this would run the command with self.env
        println!("Running: {} {:?}", self.program, self.args);
        println!("Environment: {:?}", self.env.vars());

        // Tasks can write outputs for downstream tasks
        ctx.outputs.set("exit_code", 0)?;
        ctx.outputs.set("completed", true)?;

        Ok(())
    }

    fn environment(&self) -> Environment {
        self.env.clone()
    }
}

#[test]
fn test_yaml_like_dag_construction() {
    // This demonstrates how the YAML loader would construct a DAG
    // Users write YAML, we parse it into this structure

    let dag = DagBuilder::new("etl_pipeline", "ETL Pipeline")
        .add_task(MockCommandTask::new(
            "extract",
            "python",
            vec!["scripts/extract.py", "--input", "/data/raw"],
        ))
        .add_task_with_deps(
            MockCommandTask::new("transform", "python", vec!["scripts/transform.py"]),
            &["extract"],
        )
        .add_task_with_deps(
            MockCommandTask::new("validate", "python", vec!["scripts/validate.py"]),
            &["transform"],
        )
        .add_task_with_deps(
            MockCommandTask::new("notify", "bash", vec!["-c", "echo done"]),
            &["validate"],
        )
        .build()
        .expect("DAG should be valid");

    // Verify structure
    assert_eq!(dag.len(), 4);

    // Verify topological order respects dependencies
    let order = dag.topological_sort().unwrap();
    let names: Vec<&str> = order.iter().map(|id| id.as_str()).collect();

    // extract must come first
    assert_eq!(names[0], "extract");
    // notify must come last
    assert_eq!(names[3], "notify");
}

#[test]
fn test_task_with_environment() {
    // Demonstrate how environment variables are passed to tasks

    let task_env = Environment::new()
        .with_var("DATABASE_URL", "postgres://localhost/mydb")
        .with_var("API_KEY", "secret123")
        .with_var("LOG_LEVEL", "debug");

    let task = MockCommandTask::with_env(
        "db_task",
        "python",
        vec!["query.py"],
        task_env,
    );

    let env = task.environment();

    assert_eq!(env.get("DATABASE_URL"), Some("postgres://localhost/mydb"));
    assert_eq!(env.get("API_KEY"), Some("secret123"));
    assert_eq!(env.get("LOG_LEVEL"), Some("debug"));
}

#[test]
fn test_environment_merging() {
    // Job-level environment (from YAML job definition)
    let job_env = Environment::new()
        .with_var("LOG_LEVEL", "info")
        .with_var("DATA_DIR", "/data");

    // Task-level environment (from YAML task definition)
    let task_env = Environment::new()
        .with_var("LOG_LEVEL", "debug")  // Override job-level
        .with_var("DATABASE_URL", "postgres://localhost/db");

    // When executing, job env is merged with task env (task wins)
    let merged = job_env.merged_with(&task_env);

    assert_eq!(merged.get("LOG_LEVEL"), Some("debug"));  // Task override
    assert_eq!(merged.get("DATA_DIR"), Some("/data"));    // From job
    assert_eq!(merged.get("DATABASE_URL"), Some("postgres://localhost/db")); // From task
}

#[test]
fn test_ready_tasks_simulation() {
    // Simulate how the executor would determine which tasks to run

    let dag = DagBuilder::new("pipeline", "Pipeline")
        .add_task(MockCommandTask::new("a", "echo", vec!["a"]))
        .add_task(MockCommandTask::new("b", "echo", vec!["b"]))
        .add_task_with_deps(MockCommandTask::new("c", "echo", vec!["c"]), &["a", "b"])
        .build()
        .unwrap();

    use std::collections::HashSet;

    // Initially, a and b are ready (no dependencies)
    let ready = dag.get_ready_tasks(&HashSet::new());
    assert_eq!(ready.len(), 2);

    // After a completes, still waiting for b
    let mut completed = HashSet::new();
    completed.insert(TaskId::new("a"));
    let ready = dag.get_ready_tasks(&completed);
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].as_str(), "b");

    // After both a and b complete, c is ready
    completed.insert(TaskId::new("b"));
    let ready = dag.get_ready_tasks(&completed);
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].as_str(), "c");
}

#[tokio::test]
async fn test_task_context_data_flow() {
    // Demonstrate how data flows between tasks via context

    use std::collections::HashMap;
    use std::sync::{Arc, RwLock};
    use serde_json::Value;

    // Shared context store (this is what the executor manages)
    let store = Arc::new(RwLock::new(HashMap::<String, Value>::new()));
    let config = Arc::new(HashMap::new());

    // Simulate extract task writing output
    {
        let ctx = TaskContext::new(
            store.clone(),
            TaskId::new("extract"),
            config.clone(),
        );

        // Extract task writes row count
        ctx.outputs.set("row_count", 1000).unwrap();
        ctx.outputs.set("output_path", "/tmp/extracted.parquet").unwrap();
    }

    // Simulate transform task reading from extract
    {
        let ctx = TaskContext::new(
            store.clone(),
            TaskId::new("transform"),
            config.clone(),
        );

        // Transform reads extract's output
        let row_count: i32 = ctx.inputs.get("extract.row_count").unwrap();
        let input_path: String = ctx.inputs.get("extract.output_path").unwrap();

        assert_eq!(row_count, 1000);
        assert_eq!(input_path, "/tmp/extracted.parquet");

        // Transform writes its own output
        ctx.outputs.set("transformed_rows", row_count * 2).unwrap();
    }

    // Simulate load task reading from transform
    {
        let ctx = TaskContext::new(
            store.clone(),
            TaskId::new("load"),
            config,
        );

        let transformed: i32 = ctx.inputs.get("transform.transformed_rows").unwrap();
        assert_eq!(transformed, 2000);
    }
}

#[test]
fn test_config_access() {
    // Demonstrate how job-level config is accessed by tasks

    use std::collections::HashMap;
    use std::sync::{Arc, RwLock};
    use serde_json::{json, Value};

    let store = Arc::new(RwLock::new(HashMap::<String, Value>::new()));

    // Job-level config (would come from YAML)
    let mut config = HashMap::new();
    config.insert("input_path".to_string(), json!("/data/raw"));
    config.insert("output_path".to_string(), json!("/data/processed"));
    config.insert("batch_size".to_string(), json!(1000));

    let ctx = TaskContext::new(
        store,
        TaskId::new("my_task"),
        Arc::new(config),
    );

    // Tasks can read config values
    let input: String = ctx.get_config("input_path").unwrap();
    let batch: i32 = ctx.get_config("batch_size").unwrap();

    assert_eq!(input, "/data/raw");
    assert_eq!(batch, 1000);

    // Optional config with default
    let missing: Option<String> = ctx.get_config_optional("not_set");
    assert!(missing.is_none());
}

/// Example of what a more complex diamond dependency would look like:
///
/// ```yaml
/// tasks:
///   fetch_users:
///     type: command
///     program: python
///     args: ["fetch_users.py"]
///     environment:
///       DB_HOST: users-db.internal
///
///   fetch_orders:
///     type: command
///     program: python
///     args: ["fetch_orders.py"]
///     environment:
///       DB_HOST: orders-db.internal
///
///   join_data:
///     type: command
///     program: python
///     args: ["join.py"]
///     depends_on: [fetch_users, fetch_orders]  # waits for both
///
///   generate_report:
///     type: command
///     program: python
///     args: ["report.py"]
///     depends_on: [join_data]
///     environment:
///       OUTPUT_FORMAT: pdf
/// ```
#[test]
fn test_diamond_dependency() {
    let dag = DagBuilder::new("report", "Report Pipeline")
        .add_task(MockCommandTask::new("fetch_users", "python", vec!["fetch_users.py"]))
        .add_task(MockCommandTask::new("fetch_orders", "python", vec!["fetch_orders.py"]))
        .add_task_with_deps(
            MockCommandTask::new("join_data", "python", vec!["join.py"]),
            &["fetch_users", "fetch_orders"],
        )
        .add_task_with_deps(
            MockCommandTask::new("generate_report", "python", vec!["report.py"]),
            &["join_data"],
        )
        .build()
        .unwrap();

    let order = dag.topological_sort().unwrap();

    // fetch_users and fetch_orders can run in parallel (both before join)
    let join_pos = order.iter().position(|id| id.as_str() == "join_data").unwrap();
    let users_pos = order.iter().position(|id| id.as_str() == "fetch_users").unwrap();
    let orders_pos = order.iter().position(|id| id.as_str() == "fetch_orders").unwrap();
    let report_pos = order.iter().position(|id| id.as_str() == "generate_report").unwrap();

    assert!(users_pos < join_pos);
    assert!(orders_pos < join_pos);
    assert!(join_pos < report_pos);
}
