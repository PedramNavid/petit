//! API Sketch: How users will define and run jobs via YAML
//!
//! This test demonstrates the actual user experience using petit's
//! YAML configuration and execution infrastructure.

use petit::{
    CommandTask, DagBuilder, DagExecutor, Environment, Job, JobBuilder, Schedule, Task,
    TaskCondition, TaskContext, TaskError, TaskExecutor, TaskId, YamlLoader,
};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

/// This is what a user's YAML job definition looks like:
///
/// ```yaml
/// # jobs/etl_pipeline.yaml
/// id: etl_pipeline
/// name: ETL Pipeline
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
///   - id: extract
///     type: command
///     command: python
///     args: ["scripts/extract.py", "--input", "/data/raw"]
///     environment:
///       DATABASE_URL: postgres://localhost/source
///       BATCH_SIZE: "1000"
///     retry:
///       max_attempts: 3
///       delay_secs: 60
///
///   - id: transform
///     type: command
///     command: python
///     args: ["scripts/transform.py"]
///     depends_on: [extract]
///     environment:
///       SPARK_MASTER: local[4]
///
///   - id: validate
///     type: command
///     command: python
///     args: ["scripts/validate.py"]
///     depends_on: [transform]
///     condition: all_success
///
///   - id: notify_success
///     type: command
///     command: bash
///     args: ["-c", "echo 'Pipeline completed!'"]
///     depends_on: [validate]
///     condition: all_success
///
///   - id: notify_failure
///     type: command
///     command: bash
///     args: ["-c", "echo 'Pipeline failed!'"]
///     depends_on: [validate]
///     condition: on_failure
/// ```

// A simple mock task for testing DAG construction without running commands
struct MockCommandTask {
    name: String,
    #[allow(dead_code)]
    program: String,
    #[allow(dead_code)]
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
        // Tasks can write outputs for downstream tasks
        ctx.outputs.set("exit_code", 0)?;
        ctx.outputs.set("completed", true)?;
        Ok(())
    }

    fn environment(&self) -> Environment {
        self.env.clone()
    }
}

fn create_test_context() -> TaskContext {
    let store = Arc::new(RwLock::new(HashMap::<String, Value>::new()));
    let config = Arc::new(HashMap::new());
    TaskContext::new(store, TaskId::new("test"), config)
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

    let task = MockCommandTask::with_env("db_task", "python", vec!["query.py"], task_env);

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
        .with_var("LOG_LEVEL", "debug") // Override job-level
        .with_var("DATABASE_URL", "postgres://localhost/db");

    // When executing, job env is merged with task env (task wins)
    let merged = job_env.merged_with(&task_env);

    assert_eq!(merged.get("LOG_LEVEL"), Some("debug")); // Task override
    assert_eq!(merged.get("DATA_DIR"), Some("/data")); // From job
    assert_eq!(
        merged.get("DATABASE_URL"),
        Some("postgres://localhost/db")
    ); // From task
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

    // Shared context store (this is what the executor manages)
    let store = Arc::new(RwLock::new(HashMap::<String, Value>::new()));
    let config = Arc::new(HashMap::new());

    // Simulate extract task writing output
    {
        let ctx = TaskContext::new(store.clone(), TaskId::new("extract"), config.clone());

        // Extract task writes row count
        ctx.outputs.set("row_count", 1000).unwrap();
        ctx.outputs
            .set("output_path", "/tmp/extracted.parquet")
            .unwrap();
    }

    // Simulate transform task reading from extract
    {
        let ctx = TaskContext::new(store.clone(), TaskId::new("transform"), config.clone());

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
        let ctx = TaskContext::new(store.clone(), TaskId::new("load"), config);

        let transformed: i32 = ctx.inputs.get("transform.transformed_rows").unwrap();
        assert_eq!(transformed, 2000);
    }
}

#[test]
fn test_config_access() {
    // Demonstrate how job-level config is accessed by tasks

    use serde_json::json;

    let store = Arc::new(RwLock::new(HashMap::<String, Value>::new()));

    // Job-level config (would come from YAML)
    let mut config = HashMap::new();
    config.insert("input_path".to_string(), json!("/data/raw"));
    config.insert("output_path".to_string(), json!("/data/processed"));
    config.insert("batch_size".to_string(), json!(1000));

    let ctx = TaskContext::new(store, TaskId::new("my_task"), Arc::new(config));

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
///   - id: fetch_users
///     type: command
///     command: python
///     args: ["fetch_users.py"]
///     environment:
///       DB_HOST: users-db.internal
///
///   - id: fetch_orders
///     type: command
///     command: python
///     args: ["fetch_orders.py"]
///     environment:
///       DB_HOST: orders-db.internal
///
///   - id: join_data
///     type: command
///     command: python
///     args: ["join.py"]
///     depends_on: [fetch_users, fetch_orders]
///
///   - id: generate_report
///     type: command
///     command: python
///     args: ["report.py"]
///     depends_on: [join_data]
/// ```
#[test]
fn test_diamond_dependency() {
    let dag = DagBuilder::new("report", "Report Pipeline")
        .add_task(MockCommandTask::new(
            "fetch_users",
            "python",
            vec!["fetch_users.py"],
        ))
        .add_task(MockCommandTask::new(
            "fetch_orders",
            "python",
            vec!["fetch_orders.py"],
        ))
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
    let join_pos = order
        .iter()
        .position(|id| id.as_str() == "join_data")
        .unwrap();
    let users_pos = order
        .iter()
        .position(|id| id.as_str() == "fetch_users")
        .unwrap();
    let orders_pos = order
        .iter()
        .position(|id| id.as_str() == "fetch_orders")
        .unwrap();
    let report_pos = order
        .iter()
        .position(|id| id.as_str() == "generate_report")
        .unwrap();

    assert!(users_pos < join_pos);
    assert!(orders_pos < join_pos);
    assert!(join_pos < report_pos);
}

// ============================================================================
// Tests demonstrating the actual implementation
// ============================================================================

#[test]
fn test_command_task_builder() {
    // Demonstrate the actual CommandTask builder API
    let task = CommandTask::builder("echo")
        .name("my_task")
        .arg("hello")
        .arg("world")
        .env("MY_VAR", "my_value")
        .working_dir("/tmp")
        .build();

    assert_eq!(task.name(), "my_task");
    assert_eq!(task.environment().get("MY_VAR"), Some("my_value"));
}

#[tokio::test]
async fn test_real_command_execution() {
    // Demonstrate executing a real command task
    let task: Arc<dyn Task> = Arc::new(
        CommandTask::builder("echo")
            .name("echo_test")
            .arg("Hello from petit!")
            .build(),
    );

    let dag = DagBuilder::new("simple", "Simple DAG")
        .add_task(task)
        .build()
        .unwrap();

    // Execute the DAG
    let executor = DagExecutor::with_concurrency(4);
    let mut ctx = create_test_context();
    let result = executor.execute(&dag, &mut ctx).await;

    assert!(result.success);
    assert_eq!(result.completed_count(), 1);
    assert_eq!(result.failed_count(), 0);
}

#[tokio::test]
async fn test_dag_executor_with_dependencies() {
    // Execute a DAG with dependencies using real commands
    let dag = DagBuilder::new("pipeline", "Pipeline with deps")
        .add_task(Arc::new(
            CommandTask::builder("echo")
                .name("step1")
                .arg("Step 1")
                .build(),
        ))
        .add_task_with_deps(
            Arc::new(
                CommandTask::builder("echo")
                    .name("step2")
                    .arg("Step 2")
                    .build(),
            ),
            &["step1"],
        )
        .add_task_with_deps(
            Arc::new(
                CommandTask::builder("echo")
                    .name("step3")
                    .arg("Step 3")
                    .build(),
            ),
            &["step2"],
        )
        .build()
        .unwrap();

    let executor = DagExecutor::with_concurrency(2);
    let mut ctx = create_test_context();
    let result = executor.execute(&dag, &mut ctx).await;

    assert!(result.success);
    assert_eq!(result.completed_count(), 3);
}

#[tokio::test]
async fn test_parallel_execution() {
    // Tasks without dependencies should run in parallel
    let dag = DagBuilder::new("parallel", "Parallel tasks")
        .add_task(Arc::new(
            CommandTask::builder("sleep")
                .name("task_a")
                .arg("0.1")
                .build(),
        ))
        .add_task(Arc::new(
            CommandTask::builder("sleep")
                .name("task_b")
                .arg("0.1")
                .build(),
        ))
        .add_task(Arc::new(
            CommandTask::builder("sleep")
                .name("task_c")
                .arg("0.1")
                .build(),
        ))
        .build()
        .unwrap();

    let start = std::time::Instant::now();
    let executor = DagExecutor::with_concurrency(3);
    let mut ctx = create_test_context();
    let result = executor.execute(&dag, &mut ctx).await;
    let elapsed = start.elapsed();

    assert!(result.success);
    // If run in parallel, should take ~100ms, not ~300ms
    assert!(elapsed.as_millis() < 250, "Tasks should run in parallel");
}

#[test]
fn test_yaml_loading() {
    // Test loading a job from YAML configuration
    let yaml = r#"
id: test_job
name: Test Job
schedule: "0 0 * * * *"

tasks:
  - id: greet
    type: command
    command: echo
    args: ["Hello, World!"]
    environment:
      GREETING: Hello
"#;

    let job_config = YamlLoader::parse_job_config(yaml).expect("Should parse YAML");

    assert_eq!(job_config.id, "test_job");
    assert_eq!(job_config.name, "Test Job");
    assert_eq!(job_config.tasks.len(), 1);
}

#[test]
fn test_job_builder_api() {
    // Demonstrate the Job builder API for programmatic job creation
    let dag = DagBuilder::new("my_dag", "My DAG")
        .add_task(Arc::new(
            CommandTask::builder("echo").name("task1").arg("test").build(),
        ))
        .build()
        .unwrap();

    let job = JobBuilder::new("my_job", "My Job")
        .dag(dag)
        .schedule(Schedule::new("0 */5 * * * *").unwrap()) // Every 5 minutes
        .config("job_var", "value")
        .build()
        .unwrap();

    assert_eq!(job.id().as_str(), "my_job");
    assert_eq!(job.name(), "My Job");
}

#[test]
fn test_schedule_parsing() {
    // Demonstrate cron schedule parsing
    let schedule = Schedule::new("0 30 9 * * MON-FRI").unwrap();
    assert!(schedule.next().is_ok());

    // Every minute
    let every_min = Schedule::new("0 * * * * *").unwrap();
    assert!(every_min.next().is_ok());

    // Daily at midnight
    let daily = Schedule::new("0 0 0 * * *").unwrap();
    assert!(daily.next().is_ok());
}

#[tokio::test]
async fn test_failed_task_handling() {
    // Test that DAG execution handles failures correctly
    // Note: Must use AllSuccess condition for dependent task to be skipped on upstream failure
    // (the default condition is Always, which runs regardless of upstream status)
    let dag = DagBuilder::new("failing", "Failing pipeline")
        .add_task(Arc::new(
            CommandTask::builder("false") // 'false' command returns exit code 1
                .name("will_fail")
                .build(),
        ))
        .add_task_with_deps_and_condition(
            Arc::new(
                CommandTask::builder("echo")
                    .name("never_runs")
                    .arg("This should not run")
                    .build(),
            ),
            &["will_fail"],
            TaskCondition::AllSuccess, // Skip if upstream fails
        )
        .build()
        .unwrap();

    let executor = DagExecutor::with_concurrency(2);
    let mut ctx = create_test_context();
    let result = executor.execute(&dag, &mut ctx).await;

    assert!(!result.success);
    assert_eq!(result.failed_count(), 1);
    assert_eq!(result.skipped_count(), 1);
}

#[tokio::test]
async fn test_task_output_capture() {
    // Demonstrate capturing task output
    let dag = DagBuilder::new("output", "Output capture test")
        .add_task(Arc::new(
            CommandTask::builder("echo")
                .name("producer")
                .arg("Hello from producer")
                .build(),
        ))
        .build()
        .unwrap();

    let executor = DagExecutor::with_concurrency(1);
    let mut ctx = create_test_context();
    let result = executor.execute(&dag, &mut ctx).await;

    assert!(result.success);

    // Check the task result
    let task_result = result.get_task_result(&TaskId::new("producer")).unwrap();
    assert!(task_result.success);

    // stdout is captured in context, check it
    let stdout: String = ctx.inputs.get("producer.stdout").unwrap();
    assert!(stdout.contains("Hello from producer"));
}

#[tokio::test]
async fn test_task_executor_directly() {
    // Demonstrate using TaskExecutor directly
    let executor = TaskExecutor::new(4);
    let task = CommandTask::builder("echo")
        .name("direct_task")
        .arg("direct execution")
        .build();

    let mut ctx = create_test_context();
    let result = executor.execute(&task, &mut ctx).await.unwrap();

    assert!(result.success);
    assert_eq!(result.attempts, 1);
}

#[test]
fn test_job_with_config() {
    // Demonstrate job-level configuration
    use serde_json::json;

    let dag = DagBuilder::new("config_dag", "Config DAG")
        .add_task(MockCommandTask::new("task1", "echo", vec!["test"]))
        .build()
        .unwrap();

    let job = Job::new("config_job", "Config Job", dag)
        .with_config_value("batch_size", json!(1000))
        .with_config_value("output_dir", json!("/data/output"));

    assert_eq!(job.get_config::<i32>("batch_size"), Some(1000));
    assert_eq!(
        job.get_config::<String>("output_dir"),
        Some("/data/output".to_string())
    );
}

#[test]
fn test_yaml_with_dependencies() {
    // Test YAML parsing with task dependencies
    let yaml = r#"
id: pipeline
name: Data Pipeline
tasks:
  - id: extract
    type: command
    command: python
    args: ["extract.py"]
  - id: transform
    type: command
    command: python
    args: ["transform.py"]
    depends_on: [extract]
  - id: load
    type: command
    command: python
    args: ["load.py"]
    depends_on: [transform]
"#;

    let job_config = YamlLoader::parse_job_config(yaml).expect("Should parse YAML");

    assert_eq!(job_config.tasks.len(), 3);
    assert_eq!(job_config.tasks[0].depends_on.len(), 0);
    assert_eq!(job_config.tasks[1].depends_on, vec!["extract"]);
    assert_eq!(job_config.tasks[2].depends_on, vec!["transform"]);
}

#[test]
fn test_yaml_with_retry_config() {
    // Test YAML parsing with retry configuration
    let yaml = r#"
id: retry_job
name: Retry Job
tasks:
  - id: flaky_task
    type: command
    command: ./flaky.sh
    retry:
      max_attempts: 5
      delay_secs: 30
      condition: always
"#;

    let job_config = YamlLoader::parse_job_config(yaml).expect("Should parse YAML");

    let retry = job_config.tasks[0].retry.as_ref().unwrap();
    assert_eq!(retry.max_attempts, 5);
    assert_eq!(retry.delay_secs, 30);
}
