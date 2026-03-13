//! Event processing pipeline for ClotoCore kernel.
//!
//! Receives events via an mpsc channel, enforces cascade depth limits,
//! broadcasts to SSE subscribers, maintains an event history ring buffer,
//! and dispatches to the plugin registry for MCP server processing.

use crate::handlers::system::SystemHandler;
use crate::managers::{AgentManager, PluginManager, PluginRegistry};
use cloto_shared::{ClotoEvent, Permission};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Semaphore};
use tracing::{debug, error, info, warn};

/// Interval between event history cleanup sweeps in seconds.
const EVENT_CLEANUP_INTERVAL_SECS: u64 = 300;

/// Global monotonic sequence counter for SSE event ordering.
static GLOBAL_SEQ: AtomicU64 = AtomicU64::new(1);

/// Transport-layer wrapper that pairs a `ClotoEvent` with a monotonic sequence ID.
/// Used for SSE `id:` field and `Last-Event-ID` replay, without modifying `ClotoEvent` (shared crate).
#[derive(Debug, Clone)]
pub struct SequencedEvent {
    pub seq_id: u64,
    pub event: Arc<ClotoEvent>,
}

impl SequencedEvent {
    pub fn new(event: Arc<ClotoEvent>) -> Self {
        Self {
            seq_id: GLOBAL_SEQ.fetch_add(1, Ordering::Relaxed),
            event,
        }
    }
}

pub struct EventProcessor {
    registry: Arc<PluginRegistry>,
    plugin_manager: Arc<PluginManager>,
    agent_manager: AgentManager,
    tx_internal: broadcast::Sender<SequencedEvent>,
    history: Arc<tokio::sync::RwLock<VecDeque<SequencedEvent>>>,
    metrics: Arc<crate::managers::SystemMetrics>,
    max_history_size: usize,
    event_retention_hours: u64, // M-10: Configurable retention period
    consensus: Option<Arc<crate::consensus::ConsensusOrchestrator>>,
    /// Per-plugin rate limiter for InputControl actions (bug-143: Guardrail 1.6)
    action_rate_limiter: Arc<dashmap::DashMap<String, governor::DefaultDirectRateLimiter>>,
    /// Kernel system handler — runs agentic loops outside the plugin dispatch pipeline.
    system_handler: Arc<SystemHandler>,
    /// Per-agent semaphore to serialize agentic loops for the same agent.
    agent_locks: Arc<dashmap::DashMap<String, Arc<Semaphore>>>,
    /// Maximum event history size for cleanup (count-based cap).
    max_event_history: usize,
}

impl EventProcessor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        registry: Arc<PluginRegistry>,
        plugin_manager: Arc<PluginManager>,
        agent_manager: AgentManager,
        tx_internal: broadcast::Sender<SequencedEvent>,
        history: Arc<tokio::sync::RwLock<VecDeque<SequencedEvent>>>,
        metrics: Arc<crate::managers::SystemMetrics>,
        max_history_size: usize,
        event_retention_hours: u64, // M-10: Configurable retention period
        consensus: Option<Arc<crate::consensus::ConsensusOrchestrator>>,
        system_handler: Arc<SystemHandler>,
        max_event_history: usize,
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
            action_rate_limiter: Arc::new(dashmap::DashMap::new()),
            system_handler,
            agent_locks: Arc::new(dashmap::DashMap::new()),
            max_event_history,
        }
    }

    async fn record_event(&self, seq_event: SequencedEvent) {
        let mut history = self.history.write().await;
        history.push_back(seq_event);
        // H-06: Use while loop to handle bursts that exceed capacity
        while history.len() > self.max_history_size {
            history.pop_front();
        }
    }

    pub fn spawn_cleanup_task(self: Arc<Self>, shutdown: Arc<tokio::sync::Notify>) {
        let processor = self.clone();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(EVENT_CLEANUP_INTERVAL_SECS));
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
        // M-10: Use configurable retention period instead of hardcoded 24h
        #[allow(clippy::cast_possible_wrap)]
        let cutoff =
            chrono::Utc::now() - chrono::Duration::hours(self.event_retention_hours as i64);
        let mut history = self.history.write().await;

        // Remove old events by timestamp
        while let Some(oldest) = history.front() {
            if oldest.event.timestamp < cutoff {
                history.pop_front();
            } else {
                break;
            }
        }

        // Apply count-based cap to prevent unbounded growth
        if history.len() > self.max_event_history {
            let excess = history.len() - self.max_event_history;
            for _ in 0..excess {
                history.pop_front();
            }
            tracing::warn!(
                trimmed = excess,
                retained = self.max_event_history,
                "Event history trimmed to {} entries to prevent memory growth",
                self.max_event_history
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

            // Wrap in SequencedEvent and record in history BEFORE broadcasting
            let seq_event = SequencedEvent::new(event.clone());
            self.record_event(seq_event.clone()).await;

            // Increment metrics based on event type
            if let cloto_shared::ClotoEventData::MessageReceived(_) = &event.data {
                self.metrics
                    .total_requests
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }

            // ── (1) User メッセージ → SystemHandler を spawn (プラグイン外) ──
            // Agentic loop はイベントループをブロックせず独立して実行される。
            // Per-agent Semaphore で同一エージェントへの並行処理を直列化。
            if let cloto_shared::ClotoEventData::MessageReceived(ref msg) = event.data {
                if matches!(
                    msg.source,
                    cloto_shared::MessageSource::User { .. } | cloto_shared::MessageSource::System
                ) {
                    let agent_id = msg
                        .target_agent
                        .clone()
                        .or_else(|| msg.metadata.get("target_agent_id").cloned())
                        .unwrap_or_default();
                    let sem = self
                        .agent_locks
                        .entry(agent_id)
                        .or_insert_with(|| Arc::new(Semaphore::new(1)))
                        .clone();
                    let handler = self.system_handler.clone();
                    let msg = msg.clone();
                    tokio::spawn(async move {
                        let Ok(_permit) = sem.acquire().await else {
                            return;
                        };
                        if let Err(e) = handler.handle_message(msg).await {
                            error!(error = %e, "❌ SystemHandler.handle_message error");
                        }
                    });
                }
            }

            // ── (2) 即時 SSE ブロードキャスト (dispatch_event の前) ──
            // ActionRequested / PermissionGranted は後続 match で個別処理。
            match &event.data {
                cloto_shared::ClotoEventData::ActionRequested { .. }
                | cloto_shared::ClotoEventData::PermissionGranted { .. } => {}
                _ => {
                    let _ = self.tx_internal.send(seq_event.clone());
                }
            }

            // ── (3) プラグイン配信 (SystemHandler は含まれない → 高速) ──
            self.registry
                .dispatch_event(envelope.clone(), &event_tx)
                .await;

            // ── (4) Consensus Orchestrator ──
            if let Some(ref consensus) = self.consensus {
                if let Some(response_data) = consensus.handle_event(&event).await {
                    let response_event = Arc::new(ClotoEvent::with_trace(trace_id, response_data));
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

            // ── (5) イベント固有の後処理 ──
            match &event.data {
                cloto_shared::ClotoEventData::ThoughtResponse {
                    agent_id,
                    engine_id: _,
                    content,
                    source_message_id: _,
                    ..
                } => {
                    info!(trace_id = %trace_id, agent_id = %agent_id, "🧠 Received ThoughtResponse");
                    if let Err(e) = self.agent_manager.touch_last_seen(agent_id).await {
                        error!(agent_id = %agent_id, error = %e, "Failed to update last_seen on ThoughtResponse");
                    }

                    // Create additional MessageReceived for plugin cascade
                    let msg = cloto_shared::ClotoMessage::new(
                        cloto_shared::MessageSource::Agent {
                            id: agent_id.clone(),
                        },
                        content.clone(),
                    );
                    let msg_received = Arc::new(cloto_shared::ClotoEvent::with_trace(
                        trace_id,
                        cloto_shared::ClotoEventData::MessageReceived(msg.clone()),
                    ));
                    let seq_msg = SequencedEvent::new(msg_received.clone());
                    self.record_event(seq_msg.clone()).await;
                    let _ = self.tx_internal.send(seq_msg);

                    let system_envelope = crate::EnvelopedEvent {
                        event: msg_received,
                        issuer: None,
                        correlation_id: Some(trace_id),
                        depth: envelope.depth + 1,
                    };
                    let _ = event_tx.send(system_envelope).await;
                }
                cloto_shared::ClotoEventData::ActionRequested {
                    requester,
                    action: _action,
                } => {
                    let is_valid_issuer = match &envelope.issuer {
                        Some(issuer_id) => issuer_id == requester,
                        None => true,
                    };

                    if !is_valid_issuer {
                        error!(
                            trace_id = %trace_id,
                            requester_id = %requester,
                            issuer_id = ?envelope.issuer,
                            "🚫 FORGERY DETECTED: Plugin attempted to impersonate another ID in ActionRequested"
                        );
                        continue;
                    }

                    if self.authorize(requester, Permission::InputControl).await {
                        if !self.check_action_rate(&requester.to_string()) {
                            warn!(trace_id = %trace_id, requester_id = %requester, "⚡ InputControl rate limit exceeded");
                            continue;
                        }
                        info!(trace_id = %trace_id, requester_id = %requester, "✅ Action authorized");
                        let _ = self.tx_internal.send(seq_event.clone());
                    } else {
                        error!(
                            trace_id = %trace_id,
                            requester_id = %requester,
                            "🚫 SECURITY VIOLATION: Plugin attempted Action without InputControl permission"
                        );
                    }
                }
                cloto_shared::ClotoEventData::PermissionGranted {
                    plugin_id,
                    permission,
                } => {
                    info!(
                        trace_id = %trace_id,
                        plugin_id = %plugin_id,
                        permission = %permission,
                        "Permission GRANTED to plugin"
                    );

                    // Try to parse as legacy Permission enum for plugin capability injection.
                    // MGP permission strings (e.g., "shell.execute") won't parse and are
                    // handled exclusively by the MCP permission system.
                    if let Ok(legacy_perm) = serde_json::from_value::<cloto_shared::Permission>(
                        serde_json::Value::String(permission.clone()),
                    ) {
                        let cloto_id = cloto_shared::ClotoId::from_name(plugin_id);
                        self.registry
                            .update_effective_permissions(cloto_id, legacy_perm.clone())
                            .await;

                        let reg_state = self.registry.state.read().await;
                        if let Some(plugin) = reg_state.plugins.get(plugin_id) {
                            if let Some(cap) = self
                                .plugin_manager
                                .get_capability_for_permission(&legacy_perm)
                            {
                                let plugin_id = plugin_id.clone();
                                info!(trace_id = %trace_id, plugin_id = %plugin_id, "Injecting capability");
                                let plugin = plugin.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = plugin.on_capability_injected(cap).await {
                                        error!(trace_id = %trace_id, plugin_id = %plugin_id, error = %e, "Failed to inject capability");
                                    }
                                });
                            }
                        }
                        drop(reg_state);
                    }
                }
                cloto_shared::ClotoEventData::AgentPowerChanged {
                    ref agent_id,
                    enabled,
                } => {
                    info!(
                        trace_id = %trace_id,
                        agent_id = %agent_id,
                        enabled = %enabled,
                        "🔌 Agent power state changed"
                    );
                }
                cloto_shared::ClotoEventData::ToolInvoked {
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
                }
                cloto_shared::ClotoEventData::AgenticLoopCompleted {
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
                }
                _ => {}
            }
        }
    }

    /// Per-plugin rate limiting for InputControl actions (bug-143: Guardrail 1.6).
    /// Returns `true` if the action is within rate limits, `false` if rate-limited.
    fn check_action_rate(&self, requester_id: &str) -> bool {
        use governor::{Quota, RateLimiter};
        use std::num::NonZeroU32;

        let limiter = self
            .action_rate_limiter
            .entry(requester_id.to_string())
            .or_insert_with(|| {
                RateLimiter::direct(
                    Quota::per_second(NonZeroU32::new(10).unwrap())
                        .allow_burst(NonZeroU32::new(20).unwrap()),
                )
            });
        limiter.check().is_ok()
    }

    async fn authorize(&self, requester_id: &cloto_shared::ClotoId, required: Permission) -> bool {
        let state = self.registry.state.read().await;
        if let Some(perms) = state.effective_permissions.get(requester_id) {
            return perms.contains(&required);
        }
        false
    }
}
