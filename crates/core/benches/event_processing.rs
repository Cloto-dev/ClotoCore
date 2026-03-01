// Event Processing Benchmarks
// Critical path: cloto_core/src/events.rs:72-76 (EventProcessor::record_event)
// Measures: VecDeque operations, RwLock contention, event throughput

#[allow(unused_imports)]
use cloto_shared::ClotoEvent;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;

mod helpers;

fn event_recording_single(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("event_recording_single", |b| {
        b.to_async(&runtime).iter(|| async {
            let event_history = Arc::new(RwLock::new(VecDeque::new()));
            let event = helpers::create_test_event("benchmark message".to_string());

            // Benchmark VecDeque push_back with RwLock
            // Critical path from cloto_core/src/events.rs:72-76
            let mut history = event_history.write().await;
            history.push_back(black_box((*event).clone()));
            if history.len() > 1000 {
                history.pop_front();
            }
        });
    });
}

fn event_throughput_benchmark(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("event_throughput");

    for event_count in &[100, 1_000, 10_000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(event_count),
            event_count,
            |b, &count| {
                b.to_async(&runtime).iter(|| async move {
                    let event_history = Arc::new(RwLock::new(VecDeque::new()));

                    for i in 0..count {
                        let event = helpers::create_test_event(format!("message_{}", i));
                        let mut history = event_history.write().await;
                        history.push_back((*event).clone());
                        if history.len() > 1000 {
                            history.pop_front();
                        }
                    }
                });
            },
        );
    }
    group.finish();
}

fn event_history_contention_benchmark(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("event_history_concurrent_writes", |b| {
        b.to_async(&runtime).iter(|| async {
            let event_history = Arc::new(RwLock::new(VecDeque::new()));

            // Simulate concurrent writes from multiple plugins
            let mut handles = vec![];
            for i in 0..10 {
                let history = event_history.clone();
                let handle = tokio::spawn(async move {
                    for j in 0..100 {
                        let event = helpers::create_test_event(format!("msg_{}_{}", i, j));
                        let mut h = history.write().await;
                        h.push_back((*event).clone());
                        if h.len() > 1000 {
                            h.pop_front();
                        }
                    }
                });
                handles.push(handle);
            }

            for h in handles {
                h.await.unwrap();
            }
        });
    });
}

fn event_serialization_benchmark(c: &mut Criterion) {
    let event = helpers::create_test_event("Hello, benchmark!".to_string());

    c.bench_function("event_to_json", |b| {
        b.iter(|| serde_json::to_string(black_box(&*event)).unwrap());
    });
}

criterion_group!(
    benches,
    event_recording_single,
    event_throughput_benchmark,
    event_history_contention_benchmark,
    event_serialization_benchmark
);
criterion_main!(benches);
