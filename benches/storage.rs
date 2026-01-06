//! Benchmarks for storage backends.
//!
//! Measures the performance of list operations.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use petit::core::types::{DagId, JobId, RunId, TaskId};
use petit::{InMemoryStorage, Storage, StoredJob, StoredRun, StoredTaskState};
use tokio::runtime::Runtime;

fn bench_list_jobs(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("storage_list_jobs");

    for size in [100, 500].iter() {
        group.bench_with_input(BenchmarkId::new("inmemory", size), size, |b, &size| {
            let storage = InMemoryStorage::new();
            rt.block_on(async {
                for i in 0..size {
                    let job = StoredJob::new(
                        JobId::new(format!("job_{}", i)),
                        format!("Job {}", i),
                        DagId::new(format!("dag_{}", i)),
                    );
                    storage.save_job(job).await.unwrap();
                }
            });

            b.iter(|| rt.block_on(async { storage.list_jobs().await.unwrap() }));
        });
    }

    group.finish();
}

fn bench_list_runs(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("storage_list_runs");

    for count in [100, 500].iter() {
        group.bench_with_input(BenchmarkId::new("inmemory", count), count, |b, &count| {
            let storage = InMemoryStorage::new();
            let job_id = JobId::new("job1");
            rt.block_on(async {
                for _ in 0..count {
                    let run = StoredRun::new(RunId::new(), job_id.clone());
                    storage.save_run(run).await.unwrap();
                }
            });

            b.iter(|| rt.block_on(async { storage.list_runs(&job_id, 50).await.unwrap() }));
        });
    }

    group.finish();
}

fn bench_list_task_states(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("storage_list_task_states");

    for count in [100, 500].iter() {
        group.bench_with_input(BenchmarkId::new("inmemory", count), count, |b, &count| {
            let storage = InMemoryStorage::new();
            let run_id = RunId::new();
            rt.block_on(async {
                for i in 0..count {
                    let state =
                        StoredTaskState::new(TaskId::new(format!("task_{}", i)), run_id.clone());
                    storage.save_task_state(state).await.unwrap();
                }
            });

            b.iter(|| rt.block_on(async { storage.list_task_states(&run_id).await.unwrap() }));
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_list_jobs,
    bench_list_runs,
    bench_list_task_states
);

criterion_main!(benches);
