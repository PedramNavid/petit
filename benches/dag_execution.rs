//! Benchmarks for DAG operations.
//!
//! Measures the overhead of:
//! - DAG construction and validation
//! - Topological sorting

use async_trait::async_trait;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use petit::core::context::TaskContext;
use petit::core::dag::{Dag, DagBuilder};
use petit::core::task::{Task, TaskError};
use std::sync::Arc;

/// A minimal no-op task for benchmarking DAG operations.
struct NoOpTask {
    name: String,
}

impl NoOpTask {
    fn create(name: &str) -> Arc<dyn Task> {
        Arc::new(Self {
            name: name.to_string(),
        })
    }
}

#[async_trait]
impl Task for NoOpTask {
    fn name(&self) -> &str {
        &self.name
    }

    async fn execute(&self, _ctx: &mut TaskContext) -> Result<(), TaskError> {
        Ok(())
    }
}

/// Build a linear DAG: A -> B -> C -> ... -> N
fn build_linear_dag(size: usize) -> Dag {
    let mut builder = DagBuilder::new("linear", "Linear DAG");

    for i in 0..size {
        let name = format!("task_{}", i);
        if i == 0 {
            builder = builder.add_task(NoOpTask::create(&name));
        } else {
            let prev = format!("task_{}", i - 1);
            builder = builder.add_task_with_deps(NoOpTask::create(&name), &[&prev]);
        }
    }

    builder.build().unwrap()
}

/// Build a wide DAG: A -> [B, C, D, ...] (one root, many leaves)
fn build_wide_dag(size: usize) -> Dag {
    let mut builder = DagBuilder::new("wide", "Wide DAG");
    builder = builder.add_task(NoOpTask::create("root"));

    for i in 0..size {
        let name = format!("leaf_{}", i);
        builder = builder.add_task_with_deps(NoOpTask::create(&name), &["root"]);
    }

    builder.build().unwrap()
}

/// Build a diamond DAG with layers:
///        A
///      / | \
///     B  C  D
///      \ | /
///        E
fn build_diamond_dag(width: usize) -> Dag {
    let mut builder = DagBuilder::new("diamond", "Diamond DAG");
    builder = builder.add_task(NoOpTask::create("start"));

    // Middle layer
    let mut middle_names = Vec::new();
    for i in 0..width {
        let name = format!("middle_{}", i);
        middle_names.push(name.clone());
        builder = builder.add_task_with_deps(NoOpTask::create(&name), &["start"]);
    }

    // End node depends on all middle nodes
    let refs: Vec<&str> = middle_names.iter().map(|s| s.as_str()).collect();
    builder = builder.add_task_with_deps(NoOpTask::create("end"), &refs);

    builder.build().unwrap()
}

fn bench_dag_construction(c: &mut Criterion) {
    let mut group = c.benchmark_group("dag_construction");

    for size in [100, 500].iter() {
        group.bench_with_input(BenchmarkId::new("linear", size), size, |b, &size| {
            b.iter(|| build_linear_dag(size));
        });

        group.bench_with_input(BenchmarkId::new("wide", size), size, |b, &size| {
            b.iter(|| build_wide_dag(size));
        });

        group.bench_with_input(BenchmarkId::new("diamond", size), size, |b, &size| {
            b.iter(|| build_diamond_dag(size));
        });
    }

    group.finish();
}

fn bench_topological_sort(c: &mut Criterion) {
    let mut group = c.benchmark_group("topological_sort");

    for size in [100, 500].iter() {
        let linear_dag = build_linear_dag(*size);
        group.bench_with_input(BenchmarkId::new("linear", size), &linear_dag, |b, dag| {
            b.iter(|| dag.topological_sort().unwrap());
        });

        let wide_dag = build_wide_dag(*size);
        group.bench_with_input(BenchmarkId::new("wide", size), &wide_dag, |b, dag| {
            b.iter(|| dag.topological_sort().unwrap());
        });

        let diamond_dag = build_diamond_dag(*size);
        group.bench_with_input(BenchmarkId::new("diamond", size), &diamond_dag, |b, dag| {
            b.iter(|| dag.topological_sort().unwrap());
        });
    }

    group.finish();
}

fn bench_dag_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("dag_validation");

    for size in [100, 500].iter() {
        let linear_dag = build_linear_dag(*size);
        group.bench_with_input(BenchmarkId::new("linear", size), &linear_dag, |b, dag| {
            b.iter(|| dag.validate().unwrap());
        });

        let wide_dag = build_wide_dag(*size);
        group.bench_with_input(BenchmarkId::new("wide", size), &wide_dag, |b, dag| {
            b.iter(|| dag.validate().unwrap());
        });

        let diamond_dag = build_diamond_dag(*size);
        group.bench_with_input(BenchmarkId::new("diamond", size), &diamond_dag, |b, dag| {
            b.iter(|| dag.validate().unwrap());
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_dag_construction,
    bench_topological_sort,
    bench_dag_validation
);

criterion_main!(benches);
