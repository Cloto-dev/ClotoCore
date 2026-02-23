// Plugin Dispatch Benchmarks
// Critical path: exiv_core/src/managers.rs:80-162 (PluginRegistry::dispatch_event)
// Measures: FuturesUnordered concurrency, semaphore contention, plugin execution

use async_trait::async_trait;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use exiv_shared::{
    ExivEvent, ExivEventData, Plugin, PluginCast, PluginCategory, PluginManifest, ServiceType,
};
use std::any::Any;
use std::sync::Arc;
use tokio::sync::mpsc;

mod helpers;

/// Mock plugin for benchmarking with configurable latency
#[derive(Clone)]
struct BenchPlugin {
    id: String,
    latency_ms: u64,
}

impl PluginCast for BenchPlugin {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[async_trait]
impl Plugin for BenchPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: self.id.clone(),
            name: format!("Benchmark Plugin {}", self.id),
            description: "Mock plugin for benchmarking".to_string(),
            version: "1.0.0".to_string(),
            category: PluginCategory::Tool,
            service_type: ServiceType::Skill,
            tags: vec![],
            is_active: true,
            is_configured: true,
            required_config_keys: vec![],
            action_icon: None,
            action_target: None,
            icon_data: None,
            magic_seal: 0,
            sdk_version: "0.1.0".to_string(),
            required_permissions: vec![],
            provided_capabilities: vec![],
            provided_tools: vec![],
        }
    }

    async fn on_event(&self, _event: &ExivEvent) -> anyhow::Result<Option<ExivEventData>> {
        // Simulate plugin processing time
        if self.latency_ms > 0 {
            tokio::time::sleep(tokio::time::Duration::from_millis(self.latency_ms)).await;
        }
        Ok(None) // No cascading events for benchmarks
    }
}

fn plugin_dispatch_single(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("plugin_dispatch_single", |b| {
        b.to_async(&runtime).iter(|| async {
            let state = helpers::create_bench_app_state().await;
            let (event_tx, mut event_rx) = mpsc::channel(100);

            // Register single plugin with 10ms latency by directly inserting into registry
            let plugin: Arc<dyn Plugin> = Arc::new(BenchPlugin {
                id: "bench_plugin".to_string(),
                latency_ms: 10,
            });
            {
                let mut plugins = state.registry.plugins.write().await;
                plugins.insert("bench_plugin".to_string(), plugin);
            }

            // Benchmark: dispatch event to single plugin
            let event = helpers::create_enveloped_event("benchmark message".to_string());
            state.registry.dispatch_event(event, &event_tx).await;

            // Drain channel to ensure completion
            while event_rx.try_recv().is_ok() {}
        });
    });
}

fn plugin_dispatch_concurrent(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("plugin_dispatch_concurrent");

    for plugin_count in &[5, 10, 20] {
        group.bench_with_input(
            BenchmarkId::from_parameter(plugin_count),
            plugin_count,
            |b, &count| {
                b.to_async(&runtime).iter(|| async move {
                    let state = helpers::create_bench_app_state().await;
                    let (event_tx, mut event_rx) = mpsc::channel(100);

                    // Register multiple plugins with 100ms latency
                    {
                        let mut plugins = state.registry.plugins.write().await;
                        for i in 0..count {
                            let plugin: Arc<dyn Plugin> = Arc::new(BenchPlugin {
                                id: format!("plugin_{}", i),
                                latency_ms: 100,
                            });
                            plugins.insert(format!("plugin_{}", i), plugin);
                        }
                    }

                    // Benchmark: concurrent dispatch to all plugins
                    // Critical path: FuturesUnordered with semaphore from managers.rs:101-162
                    let event = helpers::create_enveloped_event("benchmark message".to_string());
                    state.registry.dispatch_event(event, &event_tx).await;

                    // Drain channel
                    while event_rx.try_recv().is_ok() {}
                });
            },
        );
    }
    group.finish();
}

fn plugin_dispatch_no_latency(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("plugin_dispatch_no_latency");

    for plugin_count in &[5, 10, 20] {
        group.bench_with_input(
            BenchmarkId::from_parameter(plugin_count),
            plugin_count,
            |b, &count| {
                b.to_async(&runtime).iter(|| async move {
                    let state = helpers::create_bench_app_state().await;
                    let (event_tx, mut event_rx) = mpsc::channel(100);

                    // Register plugins with zero latency (measure dispatch overhead only)
                    {
                        let mut plugins = state.registry.plugins.write().await;
                        for i in 0..count {
                            let plugin: Arc<dyn Plugin> = Arc::new(BenchPlugin {
                                id: format!("plugin_{}", i),
                                latency_ms: 0,
                            });
                            plugins.insert(format!("plugin_{}", i), plugin);
                        }
                    }

                    let event = helpers::create_enveloped_event("benchmark message".to_string());
                    state.registry.dispatch_event(event, &event_tx).await;

                    while event_rx.try_recv().is_ok() {}
                });
            },
        );
    }
    group.finish();
}

fn plugin_semaphore_contention(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("plugin_semaphore_contention", |b| {
        b.to_async(&runtime).iter(|| async {
            let state = helpers::create_bench_app_state().await;
            let (event_tx, mut event_rx) = mpsc::channel(1000);

            // Register 50 plugins (at semaphore limit from managers.rs:50)
            {
                let mut plugins = state.registry.plugins.write().await;
                for i in 0..50 {
                    let plugin: Arc<dyn Plugin> = Arc::new(BenchPlugin {
                        id: format!("plugin_{}", i),
                        latency_ms: 50,
                    });
                    plugins.insert(format!("plugin_{}", i), plugin);
                }
            }

            // Dispatch multiple events concurrently to stress semaphore
            let mut handles = vec![];
            for i in 0..10 {
                let registry = state.registry.clone();
                let tx = event_tx.clone();
                let handle = tokio::spawn(async move {
                    let event = helpers::create_enveloped_event(format!("event_{}", i));
                    registry.dispatch_event(event, &tx).await;
                });
                handles.push(handle);
            }

            for h in handles {
                h.await.unwrap();
            }

            // Drain channel
            while event_rx.try_recv().is_ok() {}
        });
    });
}

fn plugin_depth_limit_benchmark(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("plugin_depth_limit_check", |b| {
        b.to_async(&runtime).iter(|| async {
            let state = helpers::create_bench_app_state().await;
            let (event_tx, _event_rx) = mpsc::channel(100);

            // Create event at maximum depth (should be dropped immediately)
            // From managers.rs:89 - max_event_depth check
            let mut event = helpers::create_enveloped_event("depth test".to_string());
            event.depth = 10; // At max depth limit

            // Should return immediately without dispatching
            state.registry.dispatch_event(event, &event_tx).await;
        });
    });
}

criterion_group!(
    benches,
    plugin_dispatch_single,
    plugin_dispatch_concurrent,
    plugin_dispatch_no_latency,
    plugin_semaphore_contention,
    plugin_depth_limit_benchmark
);
criterion_main!(benches);
