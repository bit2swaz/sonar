use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use sonar_prover::sp1_wrapper::{compute_historical_avg_result, run_historical_avg_program};

fn benchmark_historical_avg(c: &mut Criterion) {
    std::env::set_var("SP1_PROVER", "mock");

    let mut group = c.benchmark_group("historical_avg");
    group.sample_size(30);
    group.measurement_time(Duration::from_secs(5));

    for size in [8usize, 64, 512, 4096] {
        let balances = (0..size)
            .map(|index| (index as u64).wrapping_mul(97).wrapping_add(1_000))
            .collect::<Vec<_>>();
        let encoded =
            bincode::serialize(&balances).expect("historical avg bench inputs should serialize");

        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(
            BenchmarkId::new("compute_historical_avg_result", size),
            &balances,
            |b, balances| b.iter(|| compute_historical_avg_result(black_box(balances))),
        );

        group.throughput(Throughput::Bytes(encoded.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("run_historical_avg_program_mock", size),
            &encoded,
            |b, encoded| {
                b.iter(|| {
                    run_historical_avg_program(black_box(&[]), black_box(encoded))
                        .expect("mock historical avg program should succeed")
                })
            },
        );
    }

    group.finish();
}

criterion_group!(benches, benchmark_historical_avg);
criterion_main!(benches);
