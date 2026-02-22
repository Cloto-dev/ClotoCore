use crate::evolution::AgentSnapshot;
use crate::managers::{AgentManager, PluginManager, PluginRegistry};
use exiv_shared::{ExivEvent, Permission};
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, error, info, warn};

/// Build an AgentSnapshot from the PluginRegistry.
/// Shared between handlers/evolution.rs and the auto-evaluation path.
pub(crate) async fn build_snapshot_from_registry_inner(registry: &PluginRegistry) -> AgentSnapshot {
    let manifests = registry.list_plugins().await;
    let active: Vec<_> = manifests.iter().filter(|m| m.is_active).collect();
    let all_plugin_ids: Vec<String> = active.iter().map(|m| m.id.clone()).collect();
    AgentSnapshot {
        active_plugins: all_plugin_ids,
        plugin_capabilities: active
            .iter()
            .map(|m| {
                (
                    m.id.clone(),
                    m.provided_capabilities
                        .iter()
                        .map(|c| format!("{:?}", c))
                        .collect(),
                )
            })
            .collect(),
        personality_hash: String::new(),
        strategy_params: HashMap::new(),
    }
}

pub struct EventProcessor {
    registry: Arc<PluginRegistry>,
    plugin_manager: Arc<PluginManager>,
    agent_manager: AgentManager,
    tx_internal: broadcast::Sender<Arc<ExivEvent>>,
    refresh_tx: mpsc::Sender<()>, // 🔄 ルート更新用チャンネル
    history: Arc<tokio::sync::RwLock<VecDeque<Arc<ExivEvent>>>>,
    metrics: Arc<crate::managers::SystemMetrics>,
    max_history_size: usize,
    event_retention_hours: u64, // M-10: Configurable retention period
    evolution_engine: Option<Arc<crate::evolution::EvolutionEngine>>,
    fitness_collector: Option<Arc<crate::evolution::FitnessCollector>>,
}

impl EventProcessor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        registry: Arc<PluginRegistry>,
        plugin_manager: Arc<PluginManager>,
        agent_manager: AgentManager,
        tx_internal: broadcast::Sender<Arc<ExivEvent>>,
        dynamic_router: Arc<crate::DynamicRouter>,
        history: Arc<tokio::sync::RwLock<VecDeque<Arc<ExivEvent>>>>,
        metrics: Arc<crate::managers::SystemMetrics>,
        max_history_size: usize,
        event_retention_hours: u64, // M-10: Configurable retention period
        evolution_engine: Option<Arc<crate::evolution::EvolutionEngine>>,
        fitness_collector: Option<Arc<crate::evolution::FitnessCollector>>,
    ) -> Self {
        let (refresh_tx, mut refresh_rx) = mpsc::channel(1);
        let registry_clone = registry.clone();
        let dynamic_router_clone = dynamic_router.clone();

        // 🔄 デバウンスされたルート更新タスク
        tokio::spawn(async move {
            while (refresh_rx.recv().await).is_some() {
                // 連続した要求を待機してまとめる (デバウンス)
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                // チャンネルに溜まった残りのメッセージを空にする
                while refresh_rx.try_recv().is_ok() {}

                info!("🔄 Refreshing dynamic routes (debounced)...");
                let mut dynamic_routes = axum::Router::new();
                let plugins_snapshot = registry_clone.plugins.read().await;
                for (id, plugin) in plugins_snapshot.iter() {
                    if let Some(web) = plugin.as_web() {
                        dynamic_routes = web.register_routes(dynamic_routes);
                        info!("🔌 Re-registered dynamic routes for plugin: {}", id);
                    }
                }
                drop(plugins_snapshot);

                let mut router_lock = dynamic_router_clone.router.write().await;
                *router_lock = dynamic_routes;
            }
        });

        Self {
            registry,
            plugin_manager,
            agent_manager,
            tx_internal,
            refresh_tx,
            history,
            metrics,
            max_history_size,
            event_retention_hours,
            evolution_engine,
            fitness_collector,
        }
    }

    async fn request_refresh(&self) {
        let _ = self.refresh_tx.try_send(());
    }

    async fn record_event(&self, event: Arc<ExivEvent>) {
        let mut history = self.history.write().await;
        history.push_back(event);
        // H-06: Use while loop to handle bursts that exceed capacity
        while history.len() > self.max_history_size {
            history.pop_front();
        }
    }

    pub fn spawn_cleanup_task(self: Arc<Self>, shutdown: Arc<tokio::sync::Notify>) {
        let processor = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            loop {
                tokio::select! {
                    () = shutdown.notified() => {
                        tracing::info!("Event history cleanup shutting down");
                        break;
                    }
                    _ = interval.tick() => {
                        processor.cleanup_old_events().await;
                    }
                }
            }
        });
    }

    /// Spawn the active heartbeat task.
    /// Every `interval_secs` seconds, updates last_seen for all enabled agents.
    pub fn spawn_heartbeat_task(
        agent_manager: AgentManager,
        interval_secs: u64,
        shutdown: Arc<tokio::sync::Notify>,
    ) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            loop {
                tokio::select! {
                    () = shutdown.notified() => {
                        tracing::info!("Active heartbeat task shutting down");
                        break;
                    }
                    _ = interval.tick() => {
                        match agent_manager.list_agents().await {
                            Ok(agents) => {
                                let enabled_count = agents.iter().filter(|a| a.enabled).count();
                                for agent in &agents {
                                    if agent.enabled {
                                        if let Err(e) = agent_manager.touch_last_seen(&agent.id).await {
                                            error!(agent_id = %agent.id, error = %e, "Heartbeat: failed to update last_seen");
                                        }
                                    }
                                }
                                debug!("Heartbeat: pinged {} enabled agents", enabled_count);
                            }
                            Err(e) => {
                                error!("Heartbeat: failed to list agents: {}", e);
                            }
                        }
                    }
                }
            }
        });
    }

    pub async fn cleanup_old_events(&self) {
        const MAX_EVENT_HISTORY: usize = 10_000;

        // M-10: Use configurable retention period instead of hardcoded 24h
        let cutoff =
            chrono::Utc::now() - chrono::Duration::hours(self.event_retention_hours as i64);
        let mut history = self.history.write().await;

        // Remove old events by timestamp
        while let Some(oldest) = history.front() {
            if oldest.timestamp < cutoff {
                history.pop_front();
            } else {
                break;
            }
        }

        // Apply count-based cap to prevent unbounded growth
        if history.len() > MAX_EVENT_HISTORY {
            let excess = history.len() - MAX_EVENT_HISTORY;
            for _ in 0..excess {
                history.pop_front();
            }
            tracing::warn!(
                trimmed = excess,
                retained = MAX_EVENT_HISTORY,
                "Event history trimmed to {} entries to prevent memory growth",
                MAX_EVENT_HISTORY
            );
        }

        info!("Event history cleanup: {} events retained", history.len());
    }

    pub async fn process_loop(
        &self,
        mut event_rx: mpsc::Receiver<crate::EnvelopedEvent>,
        event_tx: mpsc::Sender<crate::EnvelopedEvent>,
    ) {
        info!("🧠 Kernel Event Processor Loop started.");

        while let Some(envelope) = event_rx.recv().await {
            let event = envelope.event.clone();
            let trace_id = event.trace_id;

            // Record event history
            self.record_event(event.clone()).await;

            // Fitness Collector: observe all events for automatic scoring
            if let Some(ref collector) = self.fitness_collector {
                collector.observe(&event.data).await;
            }

            // Increment metrics based on event type
            if let exiv_shared::ExivEventData::MessageReceived(_) = &event.data {
                self.metrics
                    .total_requests
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }

            // 1. 全プラグイン（および内部システムハンドラ）に配信
            self.registry
                .dispatch_event(envelope.clone(), &event_tx)
                .await;

            // 2. 内部イベント分岐処理
            match &event.data {
                exiv_shared::ExivEventData::ThoughtResponse {
                    agent_id,
                    engine_id: _,
                    content,
                    source_message_id: _,
                } => {
                    info!(trace_id = %trace_id, agent_id = %agent_id, "🧠 Received ThoughtResponse");

                    // Passive heartbeat: agent responded, update last_seen
                    self.agent_manager.touch_last_seen(agent_id).await.ok();

                    // Broadcast ThoughtResponse to SSE subscribers (dashboard needs this)
                    let _ = self.tx_internal.send(event.clone());

                    // Also create a MessageReceived for plugin cascade
                    let msg = exiv_shared::ExivMessage::new(
                        exiv_shared::MessageSource::Agent {
                            id: agent_id.clone(),
                        },
                        content.clone(),
                    );
                    let msg_received = Arc::new(exiv_shared::ExivEvent::with_trace(
                        trace_id,
                        exiv_shared::ExivEventData::MessageReceived(msg.clone()),
                    ));
                    let _ = self.tx_internal.send(msg_received.clone());

                    let system_envelope = crate::EnvelopedEvent {
                        event: msg_received,
                        issuer: None,
                        correlation_id: Some(trace_id),
                        depth: envelope.depth + 1,
                    };
                    let _ = event_tx.send(system_envelope).await;

                    // Self-Evolution: auto-evaluate or track interaction
                    if let (Some(ref collector), Some(ref evo)) =
                        (&self.fitness_collector, &self.evolution_engine)
                    {
                        // Auto-evaluation: compute scores from accumulated metrics
                        let collector = collector.clone();
                        let evo = evo.clone();
                        let registry = self.registry.clone();
                        let agent_id = agent_id.clone();
                        let tx = event_tx.clone();
                        tokio::spawn(async move {
                            let scores = collector.compute_scores(&agent_id).await;
                            let snapshot = build_snapshot_from_registry_inner(&registry).await;
                            match evo.evaluate(&agent_id, scores, snapshot).await {
                                Ok(events) => {
                                    for event_data in events {
                                        let evo_event = Arc::new(ExivEvent::new(event_data));
                                        let envelope = crate::EnvelopedEvent {
                                            event: evo_event,
                                            issuer: None,
                                            correlation_id: None,
                                            depth: 0,
                                        };
                                        let _ = tx.send(envelope).await;
                                    }
                                }
                                Err(e) => {
                                    error!(agent_id = %agent_id, error = %e, "Auto-evaluation failed");
                                }
                            }
                        });
                    } else if let Some(ref evo) = self.evolution_engine {
                        // Fallback: on_interaction only (no auto-scoring)
                        let evo = evo.clone();
                        let agent_id = agent_id.clone();
                        let tx = event_tx.clone();
                        tokio::spawn(async move {
                            match evo.on_interaction(&agent_id).await {
                                Ok(events) => {
                                    for event_data in events {
                                        let evo_event = Arc::new(ExivEvent::new(event_data));
                                        let envelope = crate::EnvelopedEvent {
                                            event: evo_event,
                                            issuer: None,
                                            correlation_id: None,
                                            depth: 0,
                                        };
                                        let _ = tx.send(envelope).await;
                                    }
                                }
                                Err(e) => {
                                    error!(agent_id = %agent_id, error = %e, "Evolution interaction tracking failed");
                                }
                            }
                        });
                    }
                }
                exiv_shared::ExivEventData::ActionRequested {
                    requester,
                    action: _action,
                } => {
                    // Security Check: Verify that the issuer matches the requester
                    let is_valid_issuer = match &envelope.issuer {
                        Some(issuer_id) => issuer_id == requester,
                        None => true, // System/Kernel can act on behalf of anyone
                    };

                    if !is_valid_issuer {
                        error!(
                            trace_id = %trace_id,
                            requester_id = %requester,
                            issuer_id = ?envelope.issuer,
                            "🚫 FORGERY DETECTED: Plugin attempted to impersonate another ID in ActionRequested"
                        );
                        continue; // Drop the event
                    }

                    if self.authorize(requester, Permission::InputControl).await {
                        info!(trace_id = %trace_id, requester_id = %requester, "✅ Action authorized");
                        let _ = self.tx_internal.send(event.clone());
                    } else {
                        error!(
                            trace_id = %trace_id,
                            requester_id = %requester,
                            "🚫 SECURITY VIOLATION: Plugin attempted Action without InputControl permission"
                        );
                    }
                }
                exiv_shared::ExivEventData::PermissionGranted {
                    plugin_id,
                    permission,
                } => {
                    info!(
                        trace_id = %trace_id,
                        plugin_id = %plugin_id,
                        permission = ?permission,
                        "🔐 Permission GRANTED to plugin"
                    );

                    // 1. 権限リストの更新 (In-memory)
                    let exiv_id = exiv_shared::ExivId::from_name(plugin_id);
                    self.registry
                        .update_effective_permissions(exiv_id, permission.clone())
                        .await;

                    // 2. Capability の注入
                    let plugins = self.registry.plugins.read().await;
                    if let Some(plugin) = plugins.get(plugin_id) {
                        if let Some(cap) = self
                            .plugin_manager
                            .get_capability_for_permission(permission)
                        {
                            let plugin_id = plugin_id.clone(); // Clone for spawn
                            info!(trace_id = %trace_id, plugin_id = %plugin_id, "💉 Injecting capability");
                            let plugin = plugin.clone();
                            tokio::spawn(async move {
                                if let Err(e) = plugin.on_capability_injected(cap).await {
                                    error!(trace_id = %trace_id, plugin_id = %plugin_id, error = %e, "❌ Failed to inject capability");
                                }
                            });
                        }
                    }
                    drop(plugins);

                    // 3. ルーティングの更新をリクエスト
                    self.request_refresh().await;
                }
                exiv_shared::ExivEventData::ConfigUpdated { .. } => {
                    // 設定変更によってルートが変わる可能性があるため更新をリクエスト
                    self.request_refresh().await;
                    let _ = self.tx_internal.send(event);
                }
                exiv_shared::ExivEventData::AgentPowerChanged {
                    ref agent_id,
                    enabled,
                } => {
                    info!(
                        trace_id = %trace_id,
                        agent_id = %agent_id,
                        enabled = %enabled,
                        "🔌 Agent power state changed"
                    );
                    let _ = self.tx_internal.send(event);
                }
                // Self-Evolution events
                exiv_shared::ExivEventData::EvolutionBreach {
                    ref agent_id,
                    ref violation_type,
                    ref detail,
                } => {
                    error!(
                        trace_id = %trace_id,
                        agent_id = %agent_id,
                        violation = %violation_type,
                        detail = %detail,
                        "🚨 EVOLUTION SAFETY BREACH — Agent stopped immediately"
                    );
                    // Stop the agent
                    if let Err(e) = self.agent_manager.set_enabled(agent_id, false).await {
                        error!(agent_id = %agent_id, error = %e, "Failed to stop agent after safety breach");
                    } else {
                        // Notify subscribers that agent power state changed
                        let power_event = Arc::new(ExivEvent::new(
                            exiv_shared::ExivEventData::AgentPowerChanged {
                                agent_id: agent_id.clone(),
                                enabled: false,
                            },
                        ));
                        let _ = self.tx_internal.send(power_event);
                    }
                    let _ = self.tx_internal.send(event);
                }
                exiv_shared::ExivEventData::EvolutionGeneration {
                    ref agent_id,
                    generation,
                    ref trigger,
                } => {
                    info!(
                        trace_id = %trace_id,
                        agent_id = %agent_id,
                        generation = generation,
                        trigger = %trigger,
                        "📈 Evolution: new generation"
                    );
                    // Reset fitness collector metrics for this agent on generation transition
                    if let Some(ref collector) = self.fitness_collector {
                        collector.reset(agent_id).await;
                    }
                    let _ = self.tx_internal.send(event);
                }
                exiv_shared::ExivEventData::EvolutionRollback {
                    ref agent_id,
                    from_generation,
                    to_generation,
                    ..
                } => {
                    warn!(
                        trace_id = %trace_id,
                        agent_id = %agent_id,
                        from = from_generation,
                        to = to_generation,
                        "🔄 Evolution: rollback executed"
                    );
                    let _ = self.tx_internal.send(event);
                }
                exiv_shared::ExivEventData::FitnessContribution {
                    ref agent_id,
                    ref axis,
                    score,
                    ref source_plugin,
                } => {
                    if let Some(ref collector) = self.fitness_collector {
                        debug!(
                            trace_id = %trace_id,
                            agent_id = %agent_id,
                            axis = %axis,
                            score = score,
                            source = %source_plugin,
                            "📊 Fitness contribution received"
                        );
                        collector.record_contribution(agent_id, axis, *score).await;
                    }
                    let _ = self.tx_internal.send(event);
                }
                exiv_shared::ExivEventData::ToolInvoked {
                    ref agent_id,
                    ref tool_name,
                    success,
                    duration_ms,
                    iteration,
                    ..
                } => {
                    info!(
                        trace_id = %trace_id,
                        agent_id = %agent_id,
                        tool = %tool_name,
                        success = success,
                        duration_ms = duration_ms,
                        iteration = iteration,
                        "🔧 Tool invoked"
                    );
                    let _ = self.tx_internal.send(event);
                }
                exiv_shared::ExivEventData::AgenticLoopCompleted {
                    ref agent_id,
                    total_iterations,
                    total_tool_calls,
                    ..
                } => {
                    info!(
                        trace_id = %trace_id,
                        agent_id = %agent_id,
                        iterations = total_iterations,
                        tool_calls = total_tool_calls,
                        "✅ Agentic loop completed"
                    );
                    let _ = self.tx_internal.send(event);
                }
                _ => {
                    // SSE等への通知 (EvolutionWarning, EvolutionCapability, EvolutionRebalance, etc.)
                    let _ = self.tx_internal.send(event);
                }
            }
        }
    }

    async fn authorize(&self, requester_id: &exiv_shared::ExivId, required: Permission) -> bool {
        let perms_lock = self.registry.effective_permissions.read().await;
        if let Some(perms) = perms_lock.get(requester_id) {
            return perms.contains(&required);
        }
        false
    }
}
