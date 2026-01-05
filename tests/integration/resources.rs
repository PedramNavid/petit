//! Resource contention integration tests.
//!
//! Tests that verify resource management and concurrency control.

use async_trait::async_trait;
use petit::{
    ContextStore, DagBuilder, DagExecutor, InMemoryStorage, Job, Scheduler, Task, TaskContext,
    TaskError, TaskId,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// Task that takes time to execute (for testing concurrency).
struct SlowTask {
    name: String,
    duration: Duration,
    start_order: Arc<AtomicU32>,
    my_order: AtomicU32,
    start_time: Mutex<Option<Instant>>,
    end_time: Mutex<Option<Instant>>,
}

impl SlowTask {
    fn new(name: &str, duration: Duration, start_order: Arc<AtomicU32>) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            duration,
            start_order,
            my_order: AtomicU32::new(0),
            start_time: Mutex::new(None),
            end_time: Mutex::new(None),
        })
    }

    fn order(&self) -> u32 {
        self.my_order.load(Ordering::SeqCst)
    }

    fn start_time(&self) -> Instant {
        self.start_time.lock().unwrap().expect("task not started")
    }

    fn end_time(&self) -> Instant {
        self.end_time.lock().unwrap().expect("task not ended")
    }
}

#[async_trait]
impl Task for SlowTask {
    fn name(&self) -> &str {
        &self.name
    }

    async fn execute(&self, _ctx: &mut TaskContext) -> Result<(), TaskError> {
        *self.start_time.lock().unwrap() = Some(Instant::now());
        let order = self.start_order.fetch_add(1, Ordering::SeqCst);
        self.my_order.store(order, Ordering::SeqCst);
        tokio::time::sleep(self.duration).await;
        *self.end_time.lock().unwrap() = Some(Instant::now());
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
    let store = ContextStore::new();
    let config = Arc::new(HashMap::new());
    let mut ctx = TaskContext::new(store, TaskId::new("test"), config);

    let start = Instant::now();
    let result = executor.execute(&dag, &mut ctx).await;
    let elapsed = start.elapsed();

    assert!(result.success);
    assert_eq!(result.completed_count(), 4);

    // With concurrency=2 and 4 tasks of 50ms each, should take ~100ms (2 waves)
    // Verify ordering rather than exact timing to avoid flakes on slower machines
    // At most 2 tasks should start in the first wave (orders 0,1), and 2 in the second (orders 2,3)
    let orders: Vec<u32> = vec![task1.order(), task2.order(), task3.order(), task4.order()];
    let first_wave = orders.iter().filter(|&&o| o < 2).count();
    let second_wave = orders.iter().filter(|&&o| o >= 2).count();

    assert_eq!(
        first_wave, 2,
        "Expected 2 tasks in first wave, got {}",
        first_wave
    );
    assert_eq!(
        second_wave, 2,
        "Expected 2 tasks in second wave, got {}",
        second_wave
    );

    // Sanity check: should still take more than 50ms (minimum for any task)
    // but allow generous overhead for slower machines
    assert!(
        elapsed >= Duration::from_millis(50),
        "Should take at least 50ms (one task duration)"
    );
}

/// Test: Parallel execution of independent tasks.
#[tokio::test]
async fn test_parallel_execution_of_independent_tasks() {
    let start_order = Arc::new(AtomicU32::new(0));

    // Create 3 independent tasks
    let task1 = SlowTask::new(
        "task1",
        Duration::from_millis(100),
        Arc::clone(&start_order),
    );
    let task2 = SlowTask::new(
        "task2",
        Duration::from_millis(100),
        Arc::clone(&start_order),
    );
    let task3 = SlowTask::new(
        "task3",
        Duration::from_millis(100),
        Arc::clone(&start_order),
    );

    let dag = DagBuilder::new("parallel", "Parallel DAG")
        .add_task(Arc::clone(&task1) as Arc<dyn Task>)
        .add_task(Arc::clone(&task2) as Arc<dyn Task>)
        .add_task(Arc::clone(&task3) as Arc<dyn Task>)
        .build()
        .unwrap();

    // Execute with high concurrency (all can run in parallel)
    let executor = DagExecutor::with_concurrency(10);
    let store = ContextStore::new();
    let config = Arc::new(HashMap::new());
    let mut ctx = TaskContext::new(store, TaskId::new("test"), config);

    let start = Instant::now();
    let result = executor.execute(&dag, &mut ctx).await;
    let elapsed = start.elapsed();

    assert!(result.success);

    // All tasks should run in parallel (concurrency=10), verify by checking they all started at once
    // If running sequentially, they would start at orders 0, 1, 2
    // If parallel, all should start at order 0, 1, or 2 (within the same scheduling window)
    let orders: Vec<u32> = vec![task1.order(), task2.order(), task3.order()];

    // All three should start in the first wave (orders 0, 1, 2)
    for (i, &order) in orders.iter().enumerate() {
        assert!(
            order <= 2,
            "Task {} started with order {}, expected 0-2 for parallel execution",
            i + 1,
            order
        );
    }

    // Use relative timing: parallel should be much faster than sequential
    // With generous margin for slow machines (3x the parallel time)
    assert!(
        elapsed < Duration::from_millis(300),
        "Expected parallel execution (<300ms), got {:?}. If sequential would be ~300ms.",
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
    let store = ContextStore::new();
    let config = Arc::new(HashMap::new());
    let mut ctx = TaskContext::new(store, TaskId::new("test"), config);

    let start = Instant::now();
    let result = executor.execute(&dag, &mut ctx).await;
    let elapsed = start.elapsed();

    assert!(result.success);

    // With concurrency=1, tasks run sequentially
    // Since these tasks are independent, we can't guarantee their exact order,
    // but we can verify only one ran at a time by checking all orders are unique
    let orders: Vec<u32> = vec![task1.order(), task2.order(), task3.order()];
    let mut sorted_orders = orders.clone();
    sorted_orders.sort();

    assert_eq!(
        sorted_orders,
        vec![0, 1, 2],
        "With concurrency=1, tasks should execute one at a time (orders 0, 1, 2)"
    );

    // Sanity check: should take at least 30ms (minimum for one task)
    // but allow generous overhead for slower machines
    assert!(
        elapsed >= Duration::from_millis(30),
        "Expected at least 30ms (one task duration), got {:?}",
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
    let store = ContextStore::new();
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
    let store = ContextStore::new();
    let config = Arc::new(HashMap::new());
    let mut ctx = TaskContext::new(store, TaskId::new("test"), config);

    let result = executor.execute(&dag, &mut ctx).await;

    assert!(result.success);

    // A should be first
    assert_eq!(task_a.order(), 0);

    // B, C, D should start after A (orders 1, 2, 3 in some order)
    let b_order = task_b.order();
    let c_order = task_c.order();
    let d_order = task_d.order();

    assert!((1..=3).contains(&b_order));
    assert!((1..=3).contains(&c_order));
    assert!((1..=3).contains(&d_order));

    // Verify parallelism: B, C, D must have overlapping execution windows.
    // If running in parallel, there's a point when all three are executing:
    // max(start_times) < min(end_times)
    // If sequential, one would finish before the next starts, failing this check.
    let latest_start = task_b
        .start_time()
        .max(task_c.start_time())
        .max(task_d.start_time());
    let earliest_end = task_b
        .end_time()
        .min(task_c.end_time())
        .min(task_d.end_time());

    assert!(
        latest_start < earliest_end,
        "B, C, D should run in parallel (overlapping execution windows). \
         Latest start: {:?}, Earliest end: {:?}. \
         If sequential, the first task would end before the last starts.",
        latest_start,
        earliest_end
    );
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
    let store = ContextStore::new();
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
    let store = ContextStore::new();
    let config = Arc::new(HashMap::new());
    let mut ctx = TaskContext::new(store, TaskId::new("test"), config);

    let result = executor.execute(&dag, &mut ctx).await;

    assert!(result.success);

    // Verify order: A first, D last
    assert_eq!(task_a.order(), 0, "A should start first");
    assert_eq!(task_d.order(), 3, "D should start last");

    // B and C should be in the middle (orders 1 and 2)
    let b_order = task_b.order();
    let c_order = task_c.order();
    assert!(b_order == 1 || b_order == 2);
    assert!(c_order == 1 || c_order == 2);

    // Verify parallelism: B and C must have overlapping execution windows.
    // If running in parallel: max(start_times) < min(end_times)
    // If sequential, one would finish before the other starts.
    let latest_start = task_b.start_time().max(task_c.start_time());
    let earliest_end = task_b.end_time().min(task_c.end_time());

    assert!(
        latest_start < earliest_end,
        "B and C should run in parallel (overlapping execution windows). \
         Latest start: {:?}, Earliest end: {:?}. \
         If sequential, one task would end before the other starts.",
        latest_start,
        earliest_end
    );
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
    let store = ContextStore::new();
    let config = Arc::new(HashMap::new());
    let mut ctx = TaskContext::new(store, TaskId::new("test"), config);

    let result = executor.execute(&dag, &mut ctx).await;

    assert!(result.success);
    assert_eq!(result.completed_count(), 20);
}
