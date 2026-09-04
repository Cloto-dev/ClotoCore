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
///
/// Wraps on `u64::MAX` overflow, which is unreachable in practice: sustaining
/// one million events per second would still take ~580,000 years to exhaust
/// the counter. SSE `Last-Event-ID` replay ordering is therefore safe for any
/// realistic kernel lifetime.
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
    /// Per-plugin rate limiter for InputControl actions (bug-143: Guardrail 1.6)
    action_rate_limiter: Arc<dashmap::DashMap<String, governor::DefaultDirectRateLimiter>>,
    /// Configured HAL rate limit (actions/sec/requester).
    hal_rate_limit_per_sec: u32,
    /// Configured HAL rate limit burst.
    hal_rate_limit_burst: u32,
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
        system_handler: Arc<SystemHandler>,
        max_event_history: usize,
        hal_rate_limit_per_sec: u32,
        hal_rate_limit_burst: u32,
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
            action_rate_limiter: Arc::new(dashmap::DashMap::new()),
            hal_rate_limit_per_sec,
            hal_rate_limit_burst,
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

            // -- (1) User message -> spawn SystemHandler (outside the plugins) --
            // The agentic loop runs independently and never blocks the event loop.
            // A per-agent semaphore serializes concurrent work for the same agent.
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

            // -- (2) Immediate SSE broadcast (before dispatch_event) --
            // ActionRequested / PermissionGranted are handled individually in the match below.
            match &event.data {
                cloto_shared::ClotoEventData::ActionRequested { .. }
                | cloto_shared::ClotoEventData::PermissionGranted { .. } => {}
                _ => {
                    let _ = self.tx_internal.send(seq_event.clone());
                }
            }

            // -- (3) Deliver to plugins (SystemHandler is not among them, so this stays fast) --
            self.registry
                .dispatch_event(envelope.clone(), &event_tx)
                .await;

            // -- (4) Event-specific post-processing --
            // (Consensus is now orchestrated in-kernel inside
            //  SystemHandler::run_consensus — see docs/CONSENSUS_REVIVAL_DESIGN.md.
            //  It no longer rides the event bus, so there is no orchestrator hook
            //  here.)
            match &event.data {
                cloto_shared::ClotoEventData::ThoughtResponse {
                    agent_id, content, ..
                } => {
                    info!(trace_id = %trace_id, agent_id = %agent_id, "🧠 Received ThoughtResponse");
                    if let Err(e) = self.agent_manager.touch_last_seen(agent_id).await {
                        error!(agent_id = %agent_id, error = %e, "Failed to update last_seen on ThoughtResponse");
                    }

                    // Create additional MessageReceived for plugin cascade.
                    // bug-487: do NOT record / SSE-broadcast this inline. It is
                    // requeued below and re-enters process_loop, which records it
                    // (L213-214) and broadcasts it (the MessageReceived SSE arm)
                    // exactly once. Recording it here too double-persisted the
                    // agent reply to /api/history and double-fired its SSE. Mirror
                    // the ExternalAction path, which only requeues the injected
                    // MessageReceived and never records it inline.
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

                    let system_envelope = crate::EnvelopedEvent {
                        event: msg_received,
                        issuer: None,
                        correlation_id: Some(trace_id),
                        depth: envelope.depth + 1,
                    };
                    // bug-457: spawn the requeue instead of awaiting `send` inline.
                    // `process_loop` is the SOLE reader of this bounded channel, so
                    // awaiting a full channel here self-deadlocks — capacity can only
                    // free via the `recv()` this very task would then be blocked from
                    // reaching. Mirrors `redispatch_plugin_event`'s `tokio::spawn`.
                    let event_tx = event_tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = event_tx.send(system_envelope).await {
                            warn!("Failed to requeue ThoughtResponse-derived message: {e}");
                        }
                    });
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
                                .get_capability_for_permission(plugin_id, &legacy_perm)
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
                cloto_shared::ClotoEventData::AgentDialogue {
                    ref caller_agent_id,
                    ref target_agent_id,
                    ref status,
                    chain_depth,
                    ..
                } => {
                    info!(
                        trace_id = %trace_id,
                        caller = %caller_agent_id,
                        target = %target_agent_id,
                        status = %status,
                        depth = chain_depth,
                        "🗣️ Agent dialogue"
                    );
                }
                // ── External Action: I/O bridge callback → agentic loop ──
                cloto_shared::ClotoEventData::McpCallbackRequested {
                    ref callback_id,
                    ref server_id,
                    ref callback_type,
                    ref message,
                    ref metadata,
                    ..
                } if callback_type == "external_message" => {
                    info!(
                        trace_id = %trace_id,
                        callback_id = %callback_id,
                        server_id = %server_id,
                        "📥 External message callback received"
                    );

                    // 1. Resolve target agent from mcp_access_control.
                    // The DB fetch is already timeout-wrapped via db_timeout at
                    // the db layer; here we only need to surface the failure
                    // reason in the log instead of dropping it silently so that
                    // "message fell back to default agent" can be traced.
                    let pool_opt = self.registry.mcp_manager.as_ref().map(|m| m.pool().clone());
                    let target_agent_id = if let Some(ref pool) = pool_opt {
                        match crate::db::mcp::get_agents_for_server(pool, server_id).await {
                            Ok(agents) => agents.into_iter().next(),
                            Err(e) => {
                                warn!(
                                    err = %e,
                                    server_id = %server_id,
                                    "Failed to resolve target agent for external message; falling back to default"
                                );
                                None
                            }
                        }
                    } else {
                        None
                    }
                    .unwrap_or_else(|| self.system_handler.default_agent_id().to_string());

                    // 2. Extract sender info from metadata
                    let meta = metadata.as_ref();
                    let sender_name = meta
                        .and_then(|m| m.get("author_name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown")
                        .to_string();
                    let source = meta
                        .and_then(|m| m.get("source"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("external")
                        .to_string();
                    let author_id = meta
                        .and_then(|m| m.get("author_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("0")
                        .to_string();

                    // 3. Build ClotoMessage with external metadata
                    let action_id = format!("ext-{}", callback_id);
                    let mut msg_metadata = std::collections::HashMap::new();
                    msg_metadata.insert("target_agent_id".into(), target_agent_id.clone());
                    msg_metadata.insert("external_action_id".into(), action_id.clone());
                    msg_metadata.insert("external_callback_id".into(), callback_id.clone());
                    msg_metadata.insert("external_source".into(), source.clone());
                    msg_metadata.insert("external_sender_name".into(), sender_name.clone());
                    msg_metadata.insert("external_server_id".into(), server_id.clone());
                    msg_metadata.insert("external_author_id".into(), author_id.clone());
                    msg_metadata.insert("skip_user_persist".into(), "true".into());

                    // Forward I/O bridge metadata so the LLM can use origin-specific tools
                    // (e.g. add_reaction needs channel_id + message_id)
                    if let Some(meta) = meta {
                        for key in ["channel_id", "message_id", "guild_id", "session_id"] {
                            if let Some(val) = meta.get(key).and_then(|v| v.as_str()) {
                                msg_metadata.insert(format!("external_{}", key), val.to_string());
                            }
                        }
                        // Forward reply reference so the LLM knows the replied-to message
                        if let Some(reference) = meta.get("reference") {
                            if !reference.is_null() {
                                if let Ok(ref_str) = serde_json::to_string(reference) {
                                    msg_metadata.insert("external_reference".into(), ref_str);
                                }
                            }
                        }
                        // Forward conversation context (short-term channel history)
                        if let Some(conv_ctx) = meta.get("conversation_context") {
                            if conv_ctx.is_array() {
                                if let Ok(ctx_str) = serde_json::to_string(conv_ctx) {
                                    msg_metadata.insert("conversation_context".into(), ctx_str);
                                }
                            }
                        }
                        // Forward tool_hint and tool_args directly (no prefix)
                        // for direct tool execution bypass in system handler
                        for key in ["tool_hint", "tool_args"] {
                            if let Some(val) = meta.get(key).and_then(|v| v.as_str()) {
                                msg_metadata.insert(key.to_string(), val.to_string());
                            }
                        }
                    }

                    let cloto_msg = cloto_shared::ClotoMessage {
                        id: cloto_shared::ClotoId::new().to_string(),
                        source: cloto_shared::MessageSource::User {
                            id: format!("{}:{}", source, author_id),
                            name: sender_name.clone(),
                        },
                        target_agent: Some(target_agent_id.clone()),
                        content: message.clone(),
                        timestamp: chrono::Utc::now(),
                        metadata: msg_metadata,
                    };

                    // 4. Resolve agent name and engine for the ExternalAction pending event
                    let (agent_name, engine_id) = self
                        .agent_manager
                        .get_agent_config(&target_agent_id)
                        .await
                        .map_or_else(
                            |_| (target_agent_id.clone(), String::new()),
                            |(meta, eng)| (meta.name, eng),
                        );

                    // 5. Emit ExternalAction "pending"
                    let pending_data = cloto_shared::ClotoEventData::ExternalAction {
                        action_id: action_id.clone(),
                        source: source.clone(),
                        source_label: source.clone(),
                        target_agent_id: target_agent_id.clone(),
                        target_agent_name: agent_name,
                        prompt: message.clone(),
                        sender_name,
                        engine_id,
                        response: None,
                        status: "pending".into(),
                        callback_id: callback_id.clone(),
                    };
                    let pending_event = Arc::new(ClotoEvent::with_trace(trace_id, pending_data));
                    let pending_seq = SequencedEvent::new(pending_event.clone());
                    self.record_event(pending_seq.clone()).await;
                    let _ = self.tx_internal.send(pending_seq);

                    // 6. Inject MessageReceived into event bus → triggers SystemHandler.handle_message()
                    let msg_event = Arc::new(ClotoEvent::with_trace(
                        trace_id,
                        cloto_shared::ClotoEventData::MessageReceived(cloto_msg),
                    ));
                    let msg_envelope = crate::EnvelopedEvent {
                        event: msg_event,
                        issuer: None,
                        correlation_id: Some(trace_id),
                        depth: envelope.depth + 1,
                    };
                    // bug-457: spawn the injection so process_loop (the sole reader
                    // of this bounded channel) never blocks on a full channel.
                    let event_tx = event_tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = event_tx.send(msg_envelope).await {
                            error!("Failed to inject external message into event bus: {}", e);
                        }
                    });
                }
                cloto_shared::ClotoEventData::ExternalAction {
                    ref action_id,
                    ref source,
                    ref target_agent_id,
                    ref status,
                    ..
                } => {
                    info!(
                        trace_id = %trace_id,
                        action_id = %action_id,
                        source = %source,
                        target = %target_agent_id,
                        status = %status,
                        "📥 External action"
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

        // AppConfig validates these are non-zero on startup, so the NonZeroU32
        // construction cannot fail here. We still guard with expect() rather
        // than unwrap() so the panic message points at the validation layer.
        let per_sec = NonZeroU32::new(self.hal_rate_limit_per_sec)
            .expect("hal_rate_limit_per_sec must be non-zero (validated in AppConfig)");
        let burst = NonZeroU32::new(self.hal_rate_limit_burst)
            .expect("hal_rate_limit_burst must be non-zero (validated in AppConfig)");

        let limiter = self
            .action_rate_limiter
            .entry(requester_id.to_string())
            .or_insert_with(|| RateLimiter::direct(Quota::per_second(per_sec).allow_burst(burst)));
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

#[cfg(test)]
mod tests {
    //! Characterization tests for the event processor.
    //!
    //! These pin the behaviour the loop has today — including its quirks — so a
    //! later rewrite for headless operation is caught changing it.

    use super::*;
    use crate::handlers::system::SystemHandler;
    use cloto_shared::{ClotoEventData, ClotoId};
    use sqlx::SqlitePool;

    /// Build an `EventProcessor` whose history / registry / metrics are also
    /// handed back so tests can assert on the state the loop mutates.
    async fn processor_with(
        max_history_size: usize,
        event_retention_hours: u64,
        max_event_history: usize,
        hal_rate_limit_per_sec: u32,
        hal_rate_limit_burst: u32,
    ) -> (
        Arc<EventProcessor>,
        Arc<tokio::sync::RwLock<VecDeque<SequencedEvent>>>,
        Arc<PluginRegistry>,
        broadcast::Receiver<SequencedEvent>,
    ) {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::init_db(&pool, "sqlite::memory:", None)
            .await
            .unwrap();

        let registry = Arc::new(PluginRegistry::new(5, 10, 50));
        let plugin_manager = Arc::new(
            crate::managers::PluginManager::new(pool.clone(), vec![], 30, 10, 50).unwrap(),
        );
        let agent_manager = AgentManager::new(pool.clone(), 90_000);
        let (tx, rx) = broadcast::channel::<SequencedEvent>(256);
        let metrics = Arc::new(crate::managers::SystemMetrics::new());
        let history = Arc::new(tokio::sync::RwLock::new(VecDeque::new()));

        let (sys_tx, _sys_rx) = mpsc::channel(16);
        let system_handler = Arc::new(SystemHandler::new(
            registry.clone(),
            agent_manager.clone(),
            "agent.test".to_string(),
            sys_tx,
            10,
            metrics.clone(),
            vec![],
            "consensus:".to_string(),
            16,
            30,
            Arc::new(dashmap::DashMap::new()),
            Arc::new(dashmap::DashMap::new()),
            pool.clone(),
            Arc::new(dashmap::DashMap::new()),
            5,
            false,
        ));

        let processor = Arc::new(EventProcessor::new(
            registry.clone(),
            plugin_manager,
            agent_manager,
            tx,
            history.clone(),
            metrics,
            max_history_size,
            event_retention_hours,
            system_handler,
            max_event_history,
            hal_rate_limit_per_sec,
            hal_rate_limit_burst,
        ));

        (processor, history, registry, rx)
    }

    fn note(text: &str) -> Arc<ClotoEvent> {
        Arc::new(ClotoEvent::new(ClotoEventData::SystemNotification(
            text.to_string(),
        )))
    }

    fn envelope(event: Arc<ClotoEvent>) -> crate::EnvelopedEvent {
        crate::EnvelopedEvent {
            event,
            issuer: None,
            correlation_id: None,
            depth: 0,
        }
    }

    /// Snapshot the `SystemNotification` payloads currently in the history.
    async fn notes_in(history: &Arc<tokio::sync::RwLock<VecDeque<SequencedEvent>>>) -> Vec<String> {
        history
            .read()
            .await
            .iter()
            .filter_map(|s| match &s.event.data {
                ClotoEventData::SystemNotification(msg) => Some(msg.clone()),
                _ => None,
            })
            .collect()
    }

    /// Poll until `cond` holds or ~2s elapse. Returns whether it held.
    async fn wait_until<F, Fut>(mut cond: F) -> bool
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        for _ in 0..200 {
            if cond().await {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        false
    }

    #[test]
    fn sequence_ids_are_strictly_increasing_for_every_new_sequenced_event() {
        // GLOBAL_SEQ is process-wide, so the absolute values depend on what
        // else ran first; only the monotonicity is a contract.
        let ids: Vec<u64> = (0..50)
            .map(|i| SequencedEvent::new(note(&format!("seq-{i}"))).seq_id)
            .collect();
        for pair in ids.windows(2) {
            assert!(
                pair[1] > pair[0],
                "sequence ids must strictly increase, got {pair:?}"
            );
        }
    }

    #[tokio::test]
    async fn recording_more_events_than_max_history_size_evicts_the_oldest_first() {
        let (processor, history, _registry, _rx) = processor_with(3, 24, 10_000, 10, 20).await;

        for i in 0..6 {
            processor
                .record_event(SequencedEvent::new(note(&format!("e{i}"))))
                .await;
        }

        assert_eq!(
            notes_in(&history).await,
            vec!["e3".to_string(), "e4".to_string(), "e5".to_string()],
            "history must hold exactly max_history_size entries, oldest evicted"
        );
    }

    #[tokio::test]
    async fn a_max_history_size_of_zero_keeps_the_history_permanently_empty() {
        // Quirk worth pinning: the cap is applied after the push, so 0 is a
        // legal (if useless) configuration rather than a panic or an off-by-one.
        let (processor, history, _registry, _rx) = processor_with(0, 24, 10_000, 10, 20).await;

        processor
            .record_event(SequencedEvent::new(note("dropped")))
            .await;

        assert!(history.read().await.is_empty());
    }

    #[tokio::test]
    async fn the_process_loop_records_every_event_it_receives_and_honours_the_history_cap() {
        let (processor, history, _registry, mut rx) = processor_with(3, 24, 10_000, 10, 20).await;

        let (tx, event_rx) = mpsc::channel::<crate::EnvelopedEvent>(32);
        for i in 0..6 {
            tx.send(envelope(note(&format!("loop-{i}")))).await.unwrap();
        }

        let loop_processor = processor.clone();
        let loop_tx = tx.clone();
        let handle = tokio::spawn(async move {
            loop_processor.process_loop(event_rx, loop_tx).await;
        });

        let history_probe = history.clone();
        assert!(
            wait_until(|| {
                let h = history_probe.clone();
                async move { h.read().await.len() == 3 }
            })
            .await,
            "the loop should have recorded all six events, capped at three"
        );

        assert_eq!(
            notes_in(&history).await,
            vec![
                "loop-3".to_string(),
                "loop-4".to_string(),
                "loop-5".to_string()
            ]
        );

        // Every event was also broadcast to SSE subscribers, with increasing ids.
        let mut seen = Vec::new();
        while let Ok(seq) = rx.try_recv() {
            seen.push(seq.seq_id);
        }
        assert_eq!(seen.len(), 6, "all six events should have been broadcast");
        for pair in seen.windows(2) {
            assert!(pair[1] > pair[0], "broadcast ids must increase: {pair:?}");
        }

        handle.abort();
    }

    #[tokio::test]
    async fn an_event_the_loop_rejects_or_cannot_parse_does_not_stop_later_events() {
        let (processor, history, _registry, _rx) = processor_with(50, 24, 10_000, 10, 20).await;

        let (tx, event_rx) = mpsc::channel::<crate::EnvelopedEvent>(32);

        // (1) A forged ActionRequested: the issuer does not match the requester,
        //     so the loop `continue`s past the rest of the body for this event.
        let forged = Arc::new(ClotoEvent::new(ClotoEventData::ActionRequested {
            requester: ClotoId::from_name("plugin.victim"),
            action: cloto_shared::HandAction::Wait { ms: 1 },
        }));
        tx.send(crate::EnvelopedEvent {
            event: forged,
            issuer: Some(ClotoId::from_name("plugin.attacker")),
            correlation_id: None,
            depth: 0,
        })
        .await
        .unwrap();

        // (2) A PermissionGranted whose permission string is not a legacy
        //     Permission — `serde_json::from_value` fails and the arm is skipped.
        tx.send(envelope(Arc::new(ClotoEvent::new(
            ClotoEventData::PermissionGranted {
                plugin_id: "plugin.ghost".to_string(),
                permission: "not::a::real::permission".to_string(),
            },
        ))))
        .await
        .unwrap();

        // (3) A well-formed event that must still be processed afterwards.
        tx.send(envelope(note("survivor"))).await.unwrap();

        let loop_processor = processor.clone();
        let loop_tx = tx.clone();
        let handle = tokio::spawn(async move {
            loop_processor.process_loop(event_rx, loop_tx).await;
        });

        let history_probe = history.clone();
        assert!(
            wait_until(|| {
                let h = history_probe.clone();
                async move { h.read().await.len() == 3 }
            })
            .await,
            "the loop must survive both malformed events and record the third"
        );
        assert_eq!(notes_in(&history).await, vec!["survivor".to_string()]);

        handle.abort();
    }

    #[tokio::test]
    async fn cleanup_drops_events_older_than_the_configured_retention_and_keeps_the_rest() {
        let (processor, history, _registry, _rx) = processor_with(1000, 1, 10_000, 10, 20).await;

        {
            let mut h = history.write().await;
            for (label, age_mins) in [("stale-a", 121_i64), ("stale-b", 90), ("fresh", 30)] {
                let mut event =
                    ClotoEvent::new(ClotoEventData::SystemNotification(label.to_string()));
                event.timestamp = chrono::Utc::now() - chrono::Duration::minutes(age_mins);
                h.push_back(SequencedEvent::new(Arc::new(event)));
            }
        }

        processor.cleanup_old_events().await;

        assert_eq!(
            notes_in(&history).await,
            vec!["fresh".to_string()],
            "retention of 1h must drop both events older than an hour"
        );
    }

    #[tokio::test]
    async fn cleanup_trims_to_max_event_history_even_when_every_event_is_recent() {
        // The count-based cap is a second, independent sweep — nothing here is
        // old enough for the timestamp sweep to touch.
        let (processor, history, _registry, _rx) = processor_with(1000, 24, 4, 10, 20).await;

        {
            let mut h = history.write().await;
            for i in 0..10 {
                h.push_back(SequencedEvent::new(note(&format!("recent-{i}"))));
            }
        }

        processor.cleanup_old_events().await;

        assert_eq!(
            notes_in(&history).await,
            vec![
                "recent-6".to_string(),
                "recent-7".to_string(),
                "recent-8".to_string(),
                "recent-9".to_string()
            ],
            "the count cap keeps the newest max_event_history entries"
        );
    }

    #[tokio::test]
    async fn the_action_rate_limiter_allows_one_burst_per_requester_and_then_refuses() {
        let (processor, _history, _registry, _rx) = processor_with(10, 24, 10_000, 1, 3).await;

        for attempt in 0..3 {
            assert!(
                processor.check_action_rate("plugin.hal"),
                "burst of 3 must admit attempt {attempt}"
            );
        }
        assert!(
            !processor.check_action_rate("plugin.hal"),
            "the fourth action within the same second must be refused"
        );
        // The limiter is per requester, so a different plugin is unaffected.
        assert!(processor.check_action_rate("plugin.other"));
    }

    #[tokio::test]
    async fn authorize_only_passes_once_the_permission_is_recorded_in_the_registry() {
        let (processor, _history, registry, _rx) = processor_with(10, 24, 10_000, 10, 20).await;
        let plugin = ClotoId::from_name("plugin.hal");

        assert!(
            !processor.authorize(&plugin, Permission::InputControl).await,
            "an unknown plugin has no effective permissions"
        );

        registry
            .update_effective_permissions(plugin, Permission::MemoryRead)
            .await;
        assert!(
            !processor.authorize(&plugin, Permission::InputControl).await,
            "holding some other permission must not grant InputControl"
        );

        registry
            .update_effective_permissions(plugin, Permission::InputControl)
            .await;
        assert!(processor.authorize(&plugin, Permission::InputControl).await);
    }

    #[tokio::test]
    async fn an_action_from_a_plugin_without_input_control_is_never_broadcast() {
        let (processor, history, registry, mut rx) = processor_with(50, 24, 10_000, 10, 20).await;
        let plugin = ClotoId::from_name("plugin.hal");

        let (tx, event_rx) = mpsc::channel::<crate::EnvelopedEvent>(32);
        tx.send(envelope(Arc::new(ClotoEvent::new(
            ClotoEventData::ActionRequested {
                requester: plugin,
                action: cloto_shared::HandAction::Wait { ms: 1 },
            },
        ))))
        .await
        .unwrap();

        let loop_processor = processor.clone();
        let loop_tx = tx.clone();
        let handle = tokio::spawn(async move {
            loop_processor.process_loop(event_rx, loop_tx).await;
        });

        let history_probe = history.clone();
        assert!(
            wait_until(|| {
                let h = history_probe.clone();
                async move { h.read().await.len() == 1 }
            })
            .await,
            "the action is still recorded in history even when unauthorized"
        );
        // Give the loop a moment in case it were (wrongly) going to broadcast.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            rx.try_recv().is_err(),
            "ActionRequested must not reach SSE subscribers without InputControl"
        );

        // Granting the permission makes the next identical action broadcast.
        registry
            .update_effective_permissions(plugin, Permission::InputControl)
            .await;
        tx.send(envelope(Arc::new(ClotoEvent::new(
            ClotoEventData::ActionRequested {
                requester: plugin,
                action: cloto_shared::HandAction::Wait { ms: 1 },
            },
        ))))
        .await
        .unwrap();

        assert!(
            wait_until(|| {
                let ok = rx.try_recv().is_ok();
                async move { ok }
            })
            .await,
            "an authorized ActionRequested must be broadcast"
        );

        handle.abort();
    }
}
