// Rate Limiting Benchmarks
// Critical path: exiv_core/src/middleware.rs:41-47 (RateLimiter::check)
// Measures: DashMap contention, token bucket operations, cleanup performance

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use exiv_core::middleware::RateLimiter;
use std::net::IpAddr;

mod helpers;

fn rate_limiter_single_ip(c: &mut Criterion) {
    c.bench_function("rate_limiter_single_ip", |b| {
        let limiter = RateLimiter::new(100, 1);
        let ip: IpAddr = "127.0.0.1".parse().unwrap();

        b.iter(|| {
            // Benchmark: DashMap lookup + token bucket check
            // Critical path from middleware.rs:41-47
            black_box(limiter.check(ip));
        });
    });
}

fn rate_limiter_multiple_ips(c: &mut Criterion) {
    let mut group = c.benchmark_group("rate_limiter_multiple_ips");

    for ip_count in &[10, 100, 1000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(ip_count),
            ip_count,
            |b, &count| {
                let limiter = RateLimiter::new(100, 1);
                let ips: Vec<IpAddr> = (0..count)
                    .map(|i| format!("192.168.{}.{}", i / 256, i % 256).parse().unwrap())
                    .collect();

                b.iter(|| {
                    // Benchmark: DashMap with concurrent IP tracking
                    for ip in &ips {
                        black_box(limiter.check(*ip));
                    }
                });
            },
        );
    }
    group.finish();
}

fn rate_limiter_concurrent_access(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("rate_limiter_concurrent_access", |b| {
        b.to_async(&runtime).iter(|| async {
            let limiter = RateLimiter::new(100, 10);
            let limiter = std::sync::Arc::new(limiter);

            // Simulate concurrent requests from 10 different IPs
            let mut handles = vec![];
            for i in 0..10 {
                let limiter = limiter.clone();
                let ip: IpAddr = format!("10.0.0.{}", i).parse().unwrap();

                let handle = tokio::spawn(async move {
                    for _ in 0..100 {
                        black_box(limiter.check(ip));
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

fn rate_limiter_cleanup_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("rate_limiter_cleanup");

    for ip_count in &[10, 100, 1000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(ip_count),
            ip_count,
            |b, &count| {
                b.iter_batched(
                    || {
                        // Setup: Create limiter with many tracked IPs
                        let limiter = RateLimiter::new(100, 1);
                        for i in 0..count {
                            let ip: IpAddr =
                                format!("172.16.{}.{}", i / 256, i % 256).parse().unwrap();
                            let _ = limiter.check(ip);
                        }
                        limiter
                    },
                    |limiter| {
                        // Benchmark: cleanup operation
                        // Critical path from middleware.rs:50-56
                        // DashMap::retain walks all shards
                        limiter.cleanup();
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }
    group.finish();
}

fn rate_limiter_tracked_ips_count(c: &mut Criterion) {
    c.bench_function("rate_limiter_tracked_ips_count", |b| {
        let limiter = RateLimiter::new(100, 1);

        // Populate with 1000 IPs
        for i in 0..1000 {
            let ip: IpAddr = format!("192.168.{}.{}", i / 256, i % 256).parse().unwrap();
            let _ = limiter.check(ip);
        }

        b.iter(|| {
            // Benchmark: DashMap length calculation
            // From middleware.rs:59-61
            black_box(limiter.tracked_ips());
        });
    });
}

fn rate_limiter_burst_behavior(c: &mut Criterion) {
    c.bench_function("rate_limiter_burst_behavior", |b| {
        let limiter = RateLimiter::new(10, 1); // Low limit for burst testing
        let ip: IpAddr = "203.0.113.42".parse().unwrap();

        b.iter(|| {
            // Benchmark: token bucket depletion
            // Should succeed for first 10 requests, then fail
            let mut allowed = 0;
            for _ in 0..20 {
                if limiter.check(ip) {
                    allowed += 1;
                }
            }
            black_box(allowed);
        });
    });
}

criterion_group!(
    benches,
    rate_limiter_single_ip,
    rate_limiter_multiple_ips,
    rate_limiter_concurrent_access,
    rate_limiter_cleanup_benchmark,
    rate_limiter_tracked_ips_count,
    rate_limiter_burst_behavior
);
criterion_main!(benches);
