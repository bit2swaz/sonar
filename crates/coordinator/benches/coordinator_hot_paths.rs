use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use sonar_coordinator::{
    callback::build_callback_instruction_data,
    listener::{
        decode_historical_avg_inputs, encode_historical_avg_inputs, parse_inputs_from_logs,
    },
};

fn make_logs(line_count: usize) -> Vec<String> {
    let mut logs = (0..line_count.saturating_sub(1))
        .map(|index| format!("Program log: noop:{index}"))
        .collect::<Vec<_>>();
    logs.push(format!("Program log: sonar:inputs:{}", "ab".repeat(48)));
    logs
}

fn bench_callback_instruction_encoding(c: &mut Criterion) {
    let mut group = c.benchmark_group("callback_instruction_encoding");
    group.sample_size(30);
    group.measurement_time(Duration::from_secs(5));

    let proof = vec![0xAB; 256];
    let public_inputs = vec![vec![0x11; 32]; 9];
    let result = vec![0xCD; 128];
    let payload_bytes =
        (proof.len() + result.len() + public_inputs.iter().map(Vec::len).sum::<usize>()) as u64;

    group.throughput(Throughput::Bytes(payload_bytes));
    group.bench_function("build_callback_instruction_data", |b| {
        b.iter(|| {
            build_callback_instruction_data(
                black_box(&proof),
                black_box(&public_inputs),
                black_box(&result),
            )
        })
    });
    group.finish();
}

fn bench_listener_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("listener_parsing");
    group.sample_size(40);
    group.measurement_time(Duration::from_secs(4));

    let historical_inputs = encode_historical_avg_inputs(&[0x42; 32], 10, 50);
    group.throughput(Throughput::Bytes(historical_inputs.len() as u64));
    group.bench_function("decode_historical_avg_inputs", |b| {
        b.iter(|| decode_historical_avg_inputs(black_box(&historical_inputs)))
    });

    for line_count in [1usize, 8, 32] {
        let logs = make_logs(line_count);
        group.throughput(Throughput::Elements(line_count as u64));
        group.bench_with_input(
            BenchmarkId::new("parse_inputs_from_logs", line_count),
            &logs,
            |b, logs| b.iter(|| parse_inputs_from_logs(black_box(logs))),
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_callback_instruction_encoding,
    bench_listener_parsing
);
criterion_main!(benches);
