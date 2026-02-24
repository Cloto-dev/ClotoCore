use crate::managers::{AgentManager, PluginManager, PluginRegistry};
use exiv_shared::{ExivEvent, Permission};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, error, info};

pub struct EventProcessor {
    registry: Arc<PluginRegistry>,
    plugin_manager: Arc<PluginManager>,
    agent_manager: AgentManager,
    tx_internal: broadcast::Sender<Arc<ExivEvent>>,
    history: Arc<tokio::sync::RwLock<VecDeque<Arc<ExivEvent>>>>,
    metrics: Arc<crate::managers::SystemMetrics>,
    max_history_size: usize,
    event_retention_hours: u64, // M-10: Configurable retention period
    consensus: Option<Arc<crate::consensus::ConsensusOrchestrator>>,
}

impl EventProcessor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        registry: Arc<PluginRegistry>,
        plugin_manager: Arc<PluginManager>,
        agent_manager: AgentManager,
        tx_internal: broadcast::Sender<Arc<ExivEvent>>,
        history: Arc<tokio::sync::RwLock<VecDeque<Arc<ExivEvent>>>>,
        metrics: Arc<crate::managers::SystemMetrics>,
        max_history_size: usize,
        event_retention_hours: u64, // M-10: Configurable retention period
        consensus: Option<Arc<crate::consensus::ConsensusOrchestrator>>,
    ) -> Self {
        Self {
            registry,
            plugin_manager,
            agent_manager,
            tx_internal,
            history,
            metrics,
            max_history_size,
            event_retention_hours,
            consensus,
        }
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
        #[allow(clippy::cast_possible_wrap)]
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

    #[allow(clippy::too_many_lines)]
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

            // 1b. Consensus Orchestrator (kernel-level, replaces core.moderator plugin)
            if let Some(ref consensus) = self.consensus {
                if let Some(response_data) = consensus.handle_event(&event).await {
                    let response_event = Arc::new(ExivEvent::with_trace(trace_id, response_data));
                    let response_envelope = crate::EnvelopedEvent {
                        event: response_event,
                        issuer: None,
                        correlation_id: Some(trace_id),
                        depth: envelope.depth + 1,
                    };
                    if let Err(e) = event_tx.send(response_envelope).await {
                        error!("Failed to send consensus response event: {}", e);
                    }
                }
            }

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
                }
                exiv_shared::ExivEventData::ConfigUpdated { .. } => {
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
                    // Forward to SSE subscribers
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
