//! Resource contention integration tests.
//!
//! Tests that verify resource management and concurrency control.

use petit::{
    DagBuilder, DagExecutor, InMemoryStorage, Job, Scheduler, Task, TaskContext, TaskError, TaskId,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Task that takes time to execute (for testing concurrency).
struct SlowTask {
    name: String,
    duration: Duration,
    start_order: Arc<AtomicU32>,
    my_order: AtomicU32,
}

impl SlowTask {
    fn new(name: &str, duration: Duration, start_order: Arc<AtomicU32>) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            duration,
            start_order,
            my_order: AtomicU32::new(0),
        })
    }

    fn order(&self) -> u32 {
        self.my_order.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Task for SlowTask {
    fn name(&self) -> &str {
        &self.name
    }

    async fn execute(&self, _ctx: &mut TaskContext) -> Result<(), TaskError> {
        let order = self.start_order.fetch_add(1, Ordering::SeqCst);
        self.my_order.store(order, Ordering::SeqCst);
        tokio::time::sleep(self.duration).await;
        Ok(())
    }
}

/// Simple success task.
struct SuccessTask {
    name: String,
}

impl SuccessTask {
    fn new(name: &str) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
        })
    }
}

#[async_trait]
impl Task for SuccessTask {
    fn name(&self) -> &str {
        &self.name
    }

    async fn execute(&self, _ctx: &mut TaskContext) -> Result<(), TaskError> {
        Ok(())
    }
}

/// Test: Concurrency limit is respected in DAG execution.
#[tokio::test]
async fn test_dag_executor_concurrency_limit() {
    let start_order = Arc::new(AtomicU32::new(0));

    // Create 4 independent tasks
    let task1 = SlowTask::new("task1", Duration::from_millis(50), Arc::clone(&start_order));
    let task2 = SlowTask::new("task2", Duration::from_millis(50), Arc::clone(&start_order));
    let task3 = SlowTask::new("task3", Duration::from_millis(50), Arc::clone(&start_order));
    let task4 = SlowTask::new("task4", Duration::from_millis(50), Arc::clone(&start_order));

    let dag = DagBuilder::new("concurrent", "Concurrent DAG")
        .add_task(Arc::clone(&task1) as Arc<dyn Task>)
        .add_task(Arc::clone(&task2) as Arc<dyn Task>)
        .add_task(Arc::clone(&task3) as Arc<dyn Task>)
        .add_task(Arc::clone(&task4) as Arc<dyn Task>)
        .build()
        .unwrap();

    // Execute with concurrency limit of 2
    let executor = DagExecutor::with_concurrency(2);
    let store = Arc::new(std::sync::RwLock::new(HashMap::new()));
    let config = Arc::new(HashMap::new());
    let mut ctx = TaskContext::new(store, TaskId::new("test"), config);

    let start = Instant::now();
    let result = executor.execute(&dag, &mut ctx).await;
    let elapsed = start.elapsed();

    assert!(result.success);
    assert_eq!(result.completed_count(), 4);

    // With concurrency=2 and 4 tasks of 50ms each, should take ~100ms (2 waves)
    // Allow some buffer for scheduling overhead
    assert!(
        elapsed >= Duration::from_millis(80),
        "Should take at least 80ms with concurrency limit"
    );
}

/// Test: Parallel execution of independent tasks.
#[tokio::test]
async fn test_parallel_execution_of_independent_tasks() {
    let start_order = Arc::new(AtomicU32::new(0));

    // Create 3 independent tasks
    let task1 = SlowTask::new("task1", Duration::from_millis(100), Arc::clone(&start_order));
    let task2 = SlowTask::new("task2", Duration::from_millis(100), Arc::clone(&start_order));
    let task3 = SlowTask::new("task3", Duration::from_millis(100), Arc::clone(&start_order));

    let dag = DagBuilder::new("parallel", "Parallel DAG")
        .add_task(Arc::clone(&task1) as Arc<dyn Task>)
        .add_task(Arc::clone(&task2) as Arc<dyn Task>)
        .add_task(Arc::clone(&task3) as Arc<dyn Task>)
        .build()
        .unwrap();

    // Execute with high concurrency (all can run in parallel)
    let executor = DagExecutor::with_concurrency(10);
    let store = Arc::new(std::sync::RwLock::new(HashMap::new()));
    let config = Arc::new(HashMap::new());
    let mut ctx = TaskContext::new(store, TaskId::new("test"), config);

    let start = Instant::now();
    let result = executor.execute(&dag, &mut ctx).await;
    let elapsed = start.elapsed();

    assert!(result.success);

    // All tasks should run in parallel, so total time should be ~100ms, not 300ms
    assert!(
        elapsed < Duration::from_millis(200),
        "Expected parallel execution (~100ms), got {:?}",
        elapsed
    );
}

/// Test: Sequential execution when concurrency is 1.
#[tokio::test]
async fn test_sequential_execution_with_concurrency_one() {
    let start_order = Arc::new(AtomicU32::new(0));

    let task1 = SlowTask::new("task1", Duration::from_millis(30), Arc::clone(&start_order));
    let task2 = SlowTask::new("task2", Duration::from_millis(30), Arc::clone(&start_order));
    let task3 = SlowTask::new("task3", Duration::from_millis(30), Arc::clone(&start_order));

    let dag = DagBuilder::new("sequential", "Sequential DAG")
        .add_task(Arc::clone(&task1) as Arc<dyn Task>)
        .add_task(Arc::clone(&task2) as Arc<dyn Task>)
        .add_task(Arc::clone(&task3) as Arc<dyn Task>)
        .build()
        .unwrap();

    // Execute with concurrency=1
    let executor = DagExecutor::with_concurrency(1);
    let store = Arc::new(std::sync::RwLock::new(HashMap::new()));
    let config = Arc::new(HashMap::new());
    let mut ctx = TaskContext::new(store, TaskId::new("test"), config);

    let start = Instant::now();
    let result = executor.execute(&dag, &mut ctx).await;
    let elapsed = start.elapsed();

    assert!(result.success);

    // With concurrency=1, tasks run sequentially: ~90ms total
    assert!(
        elapsed >= Duration::from_millis(80),
        "Expected sequential execution (~90ms), got {:?}",
        elapsed
    );
}

/// Test: Job-level concurrency limit.
#[tokio::test]
async fn test_job_level_concurrency_limit() {
    let storage = InMemoryStorage::new();
    let mut scheduler = Scheduler::new(storage);

    // Create a slow job
    struct SlowJobTask;

    #[async_trait]
    impl Task for SlowJobTask {
        fn name(&self) -> &str {
            "slow"
        }

        async fn execute(&self, _ctx: &mut TaskContext) -> Result<(), TaskError> {
            tokio::time::sleep(Duration::from_millis(300)).await;
            Ok(())
        }
    }

    let dag = DagBuilder::new("slow_dag", "Slow DAG")
        .add_task(Arc::new(SlowJobTask) as Arc<dyn Task>)
        .build()
        .unwrap();

    // Job allows only 1 concurrent run
    let job = Job::new("slow_job", "Slow Job", dag).with_max_concurrency(1);
    scheduler.register(job);

    let (handle, task) = scheduler.start().await;

    // First trigger should succeed
    let run1 = handle.trigger("slow_job").await.unwrap();
    assert!(!run1.as_uuid().is_nil());

    // Second trigger should fail (first is still running)
    let result = handle.trigger("slow_job").await;
    assert!(
        result.is_err(),
        "Second trigger should fail due to concurrency limit"
    );

    handle.shutdown().await.unwrap();
    let _ = task.await;
}

/// Test: Global scheduler concurrency limit.
#[tokio::test]
async fn test_global_scheduler_concurrency_limit() {
    let storage = InMemoryStorage::new();

    // Create slow task
    struct SlowJobTask;

    #[async_trait]
    impl Task for SlowJobTask {
        fn name(&self) -> &str {
            "slow"
        }

        async fn execute(&self, _ctx: &mut TaskContext) -> Result<(), TaskError> {
            tokio::time::sleep(Duration::from_millis(300)).await;
            Ok(())
        }
    }

    let mut scheduler = Scheduler::new(storage).with_max_concurrent_jobs(1);

    // Register two different jobs
    let dag1 = DagBuilder::new("dag1", "DAG 1")
        .add_task(Arc::new(SlowJobTask) as Arc<dyn Task>)
        .build()
        .unwrap();
    scheduler.register(Job::new("job1", "Job 1", dag1));

    let dag2 = DagBuilder::new("dag2", "DAG 2")
        .add_task(Arc::new(SlowJobTask) as Arc<dyn Task>)
        .build()
        .unwrap();
    scheduler.register(Job::new("job2", "Job 2", dag2));

    let (handle, task) = scheduler.start().await;

    // First job should succeed
    handle.trigger("job1").await.unwrap();

    // Second job (different job) should fail due to global limit
    let result = handle.trigger("job2").await;
    assert!(
        result.is_err(),
        "Second job should fail due to global concurrency limit"
    );

    handle.shutdown().await.unwrap();
    let _ = task.await;
}

/// Test: DAG with dependencies runs in correct order.
#[tokio::test]
async fn test_dependency_order_is_respected() {
    let start_order = Arc::new(AtomicU32::new(0));

    // A -> B -> C (linear dependency)
    let task_a = SlowTask::new("A", Duration::from_millis(20), Arc::clone(&start_order));
    let task_b = SlowTask::new("B", Duration::from_millis(20), Arc::clone(&start_order));
    let task_c = SlowTask::new("C", Duration::from_millis(20), Arc::clone(&start_order));

    let dag = DagBuilder::new("linear", "Linear DAG")
        .add_task(Arc::clone(&task_a) as Arc<dyn Task>)
        .add_task_with_deps(Arc::clone(&task_b) as Arc<dyn Task>, &["A"])
        .add_task_with_deps(Arc::clone(&task_c) as Arc<dyn Task>, &["B"])
        .build()
        .unwrap();

    let executor = DagExecutor::with_concurrency(10);
    let store = Arc::new(std::sync::RwLock::new(HashMap::new()));
    let config = Arc::new(HashMap::new());
    let mut ctx = TaskContext::new(store, TaskId::new("test"), config);

    let result = executor.execute(&dag, &mut ctx).await;

    assert!(result.success);

    // Tasks should have started in order: A=0, B=1, C=2
    assert_eq!(task_a.order(), 0, "A should start first");
    assert_eq!(task_b.order(), 1, "B should start second");
    assert_eq!(task_c.order(), 2, "C should start third");
}

/// Test: Fan-out pattern (one task triggers multiple).
#[tokio::test]
async fn test_fan_out_pattern() {
    let start_order = Arc::new(AtomicU32::new(0));

    //     A
    //   / | \
    //  B  C  D

    let task_a = SlowTask::new("A", Duration::from_millis(20), Arc::clone(&start_order));
    let task_b = SlowTask::new("B", Duration::from_millis(50), Arc::clone(&start_order));
    let task_c = SlowTask::new("C", Duration::from_millis(50), Arc::clone(&start_order));
    let task_d = SlowTask::new("D", Duration::from_millis(50), Arc::clone(&start_order));

    let dag = DagBuilder::new("fanout", "Fan-out DAG")
        .add_task(Arc::clone(&task_a) as Arc<dyn Task>)
        .add_task_with_deps(Arc::clone(&task_b) as Arc<dyn Task>, &["A"])
        .add_task_with_deps(Arc::clone(&task_c) as Arc<dyn Task>, &["A"])
        .add_task_with_deps(Arc::clone(&task_d) as Arc<dyn Task>, &["A"])
        .build()
        .unwrap();

    let executor = DagExecutor::with_concurrency(10);
    let store = Arc::new(std::sync::RwLock::new(HashMap::new()));
    let config = Arc::new(HashMap::new());
    let mut ctx = TaskContext::new(store, TaskId::new("test"), config);

    let start = Instant::now();
    let result = executor.execute(&dag, &mut ctx).await;
    let elapsed = start.elapsed();

    assert!(result.success);

    // A runs first (20ms), then B, C, D run in parallel (~50ms)
    // Total should be ~70ms, not 170ms
    assert!(
        elapsed < Duration::from_millis(150),
        "Fan-out should be parallel, got {:?}",
        elapsed
    );

    // A should be first
    assert_eq!(task_a.order(), 0);

    // B, C, D should start after A (orders 1, 2, 3 in some order)
    let b_order = task_b.order();
    let c_order = task_c.order();
    let d_order = task_d.order();

    assert!(b_order >= 1 && b_order <= 3);
    assert!(c_order >= 1 && c_order <= 3);
    assert!(d_order >= 1 && d_order <= 3);
}

/// Test: Fan-in pattern (multiple tasks feed into one).
#[tokio::test]
async fn test_fan_in_pattern() {
    let start_order = Arc::new(AtomicU32::new(0));

    //  A  B  C
    //   \ | /
    //     D

    let task_a = SlowTask::new("A", Duration::from_millis(30), Arc::clone(&start_order));
    let task_b = SlowTask::new("B", Duration::from_millis(50), Arc::clone(&start_order));
    let task_c = SlowTask::new("C", Duration::from_millis(40), Arc::clone(&start_order));
    let task_d = SlowTask::new("D", Duration::from_millis(20), Arc::clone(&start_order));

    let dag = DagBuilder::new("fanin", "Fan-in DAG")
        .add_task(Arc::clone(&task_a) as Arc<dyn Task>)
        .add_task(Arc::clone(&task_b) as Arc<dyn Task>)
        .add_task(Arc::clone(&task_c) as Arc<dyn Task>)
        .add_task_with_deps(Arc::clone(&task_d) as Arc<dyn Task>, &["A", "B", "C"])
        .build()
        .unwrap();

    let executor = DagExecutor::with_concurrency(10);
    let store = Arc::new(std::sync::RwLock::new(HashMap::new()));
    let config = Arc::new(HashMap::new());
    let mut ctx = TaskContext::new(store, TaskId::new("test"), config);

    let result = executor.execute(&dag, &mut ctx).await;

    assert!(result.success);

    // D should start last (after A, B, C all complete)
    assert_eq!(task_d.order(), 3, "D should start after A, B, C");
}

/// Test: Mixed fan-out and fan-in (diamond pattern).
#[tokio::test]
async fn test_diamond_pattern() {
    let start_order = Arc::new(AtomicU32::new(0));

    //     A
    //    / \
    //   B   C
    //    \ /
    //     D

    let task_a = SlowTask::new("A", Duration::from_millis(20), Arc::clone(&start_order));
    let task_b = SlowTask::new("B", Duration::from_millis(30), Arc::clone(&start_order));
    let task_c = SlowTask::new("C", Duration::from_millis(40), Arc::clone(&start_order));
    let task_d = SlowTask::new("D", Duration::from_millis(20), Arc::clone(&start_order));

    let dag = DagBuilder::new("diamond", "Diamond DAG")
        .add_task(Arc::clone(&task_a) as Arc<dyn Task>)
        .add_task_with_deps(Arc::clone(&task_b) as Arc<dyn Task>, &["A"])
        .add_task_with_deps(Arc::clone(&task_c) as Arc<dyn Task>, &["A"])
        .add_task_with_deps(Arc::clone(&task_d) as Arc<dyn Task>, &["B", "C"])
        .build()
        .unwrap();

    let executor = DagExecutor::with_concurrency(10);
    let store = Arc::new(std::sync::RwLock::new(HashMap::new()));
    let config = Arc::new(HashMap::new());
    let mut ctx = TaskContext::new(store, TaskId::new("test"), config);

    let start = Instant::now();
    let result = executor.execute(&dag, &mut ctx).await;
    let elapsed = start.elapsed();

    assert!(result.success);

    // A (20ms) -> B||C (max 40ms) -> D (20ms) = ~80ms total
    assert!(
        elapsed < Duration::from_millis(150),
        "Diamond should exploit parallelism, got {:?}",
        elapsed
    );

    // Verify order: A first, D last
    assert_eq!(task_a.order(), 0, "A should start first");
    assert_eq!(task_d.order(), 3, "D should start last");

    // B and C should be in the middle (orders 1 and 2)
    let b_order = task_b.order();
    let c_order = task_c.order();
    assert!(b_order == 1 || b_order == 2);
    assert!(c_order == 1 || c_order == 2);
}

/// Test: Large DAG with many tasks.
#[tokio::test]
async fn test_large_dag_execution() {
    let mut builder = DagBuilder::new("large", "Large DAG");

    // Create 20 tasks in a chain: task_0 -> task_1 -> ... -> task_19
    for i in 0..20 {
        let task = SuccessTask::new(&format!("task_{}", i));
        if i == 0 {
            builder = builder.add_task(task);
        } else {
            builder = builder.add_task_with_deps(task, &[&format!("task_{}", i - 1)]);
        }
    }

    let dag = builder.build().unwrap();

    let executor = DagExecutor::with_concurrency(4);
    let store = Arc::new(std::sync::RwLock::new(HashMap::new()));
    let config = Arc::new(HashMap::new());
    let mut ctx = TaskContext::new(store, TaskId::new("test"), config);

    let result = executor.execute(&dag, &mut ctx).await;

    assert!(result.success);
    assert_eq!(result.completed_count(), 20);
}
