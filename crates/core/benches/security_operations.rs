// Security Operations Benchmarks
// Critical paths:
// - cloto_core/src/capabilities.rs:54-56 (SafeHttpClient whitelist check via send_http_request)
// - cloto_core/src/handlers.rs:27 (constant-time comparison)

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::collections::HashSet;

mod helpers;

fn hashset_lookup_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("security_hashset_lookup");

    // Test with various HashSet sizes to verify O(1) behavior
    // This mirrors the internal implementation of SafeHttpClient::is_whitelisted_host
    for size in &[10, 100, 1000, 10000] {
        let hosts: HashSet<String> = (0..*size)
            .map(|i| format!("host{}.example.com", i))
            .collect();

        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, _| {
            b.iter(|| {
                // Best case: first element
                black_box(hosts.contains("host0.example.com"));
                // Worst case: last element (still O(1) for HashSet)
                black_box(hosts.contains(&format!("host{}.example.com", size - 1)));
                // Miss case
                black_box(hosts.contains("notfound.com"));
            });
        });
    }
    group.finish();
}

fn hashset_case_insensitive(c: &mut Criterion) {
    let hosts: HashSet<String> = vec![
        "api.example.com".to_lowercase(),
        "cdn.example.com".to_lowercase(),
    ]
    .into_iter()
    .collect();

    c.bench_function("security_hashset_case_insensitive", |b| {
        b.iter(|| {
            // Case-insensitive lookup (converted to lowercase before check)
            black_box(hosts.contains(&"api.example.com".to_lowercase()));
            black_box(hosts.contains(&"API.EXAMPLE.COM".to_lowercase()));
            black_box(hosts.contains(&"CDN.EXAMPLE.COM".to_lowercase()));
        });
    });
}

fn constant_time_comparison_benchmark(c: &mut Criterion) {
    use subtle::ConstantTimeEq;

    c.bench_function("security_constant_time_compare", |b| {
        b.iter(|| {
            let key1 = b"test-secret-key-12345678901234567890";
            let key2 = b"test-secret-key-12345678901234567890";

            // Benchmark constant-time comparison (authentication)
            // From cloto_core/src/handlers.rs:27
            let result = key1.ct_eq(key2);
            black_box(result);
        });
    });
}

fn constant_time_comparison_mismatch(c: &mut Criterion) {
    use subtle::ConstantTimeEq;

    c.bench_function("security_constant_time_compare_mismatch", |b| {
        b.iter(|| {
            let key1 = b"test-secret-key-12345678901234567890";
            let key2 = b"wrong-secret-key-12345678901234567890";

            // Should take same time even when keys don't match
            let result = key1.ct_eq(key2);
            black_box(result);
        });
    });
}

criterion_group!(
    benches,
    hashset_lookup_benchmark,
    hashset_case_insensitive,
    constant_time_comparison_benchmark,
    constant_time_comparison_mismatch
);
criterion_main!(benches);
