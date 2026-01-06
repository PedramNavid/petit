//! Benchmarks for schedule next-run calculations.

use chrono::{TimeZone, Utc};
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use petit::Schedule;

fn bench_next_n_occurrences(c: &mut Criterion) {
    let mut group = c.benchmark_group("next_n_occurrences");

    let base_time = Utc.with_ymd_and_hms(2024, 6, 15, 12, 0, 0).unwrap();

    let every_minute = Schedule::new("* * * * *").unwrap();
    let interval_5m = Schedule::new("@every 5m").unwrap();

    for n in [10, 50, 100].iter() {
        group.bench_with_input(BenchmarkId::new("cron_minute", n), n, |b, &n| {
            b.iter(|| every_minute.next_n_after(base_time, n).unwrap());
        });

        group.bench_with_input(BenchmarkId::new("interval_5m", n), n, |b, &n| {
            b.iter(|| interval_5m.next_n_after(base_time, n).unwrap());
        });
    }

    group.finish();
}

criterion_group!(benches, bench_next_n_occurrences);

criterion_main!(benches);
