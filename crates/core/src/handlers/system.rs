use chrono::Utc;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};

/// Maximum number of tool call entries kept in the agentic loop history.
const MAX_TOOL_HISTORY: usize = 100;

/// Number of unarchived memories that triggers automatic episode archival.
const TOOL_USAGE_THRESHOLD: usize = 10;

use crate::managers::{AgentManager, McpClientManager, PluginRegistry};
use cloto_shared::{
    AgentMetadata, ClotoEvent, ClotoEventData, ClotoId, ClotoMessage, Plugin, ThinkResult, ToolCall,
};
use sqlx::SqlitePool;

use super::command_approval::{self, PendingApprovals, SessionTrustedCommands};
use super::engine_routing::{
    evaluate_engine_routing, is_retriable_error, needs_escalation, EngineSelection,
};

pub struct SystemHandler {
    registry: Arc<PluginRegistry>,
    agent_manager: AgentManager,
    default_agent_id: String,
    sender: tokio::sync::mpsc::Sender<crate::EnvelopedEvent>,
    memory_context_limit: usize,
    metrics: Arc<crate::managers::SystemMetrics>,
    consensus_engines: Vec<String>,
    max_agentic_iterations: u8,
    tool_execution_timeout_secs: u64,
    pending_approvals: PendingApprovals,
    session_trusted_commands: SessionTrustedCommands,
    pool: SqlitePool,
    active_cron_contexts: crate::ActiveCronContexts,
    memory_timeout_secs: u64,
}

impl SystemHandler {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        registry: Arc<PluginRegistry>,
        agent_manager: AgentManager,
        default_agent_id: String,
        sender: tokio::sync::mpsc::Sender<crate::EnvelopedEvent>,
        memory_context_limit: usize,
        metrics: Arc<crate::managers::SystemMetrics>,
        consensus_engines: Vec<String>,
        max_agentic_iterations: u8,
        tool_execution_timeout_secs: u64,
        pending_approvals: PendingApprovals,
        session_trusted_commands: SessionTrustedCommands,
        pool: SqlitePool,
        active_cron_contexts: crate::ActiveCronContexts,
        memory_timeout_secs: u64,
    ) -> Self {
        Self {
            registry,
            agent_manager,
            default_agent_id,
            sender,
            memory_context_limit,
            metrics,
            consensus_engines,
            max_agentic_iterations,
            tool_execution_timeout_secs,
            pending_approvals,
            session_trusted_commands,
            pool,
            active_cron_contexts,
            memory_timeout_secs,
        }
    }

    /// Extract user_id from a ClotoMessage's source.
    fn extract_user_id(msg: &ClotoMessage) -> &str {
        match &msg.source {
            cloto_shared::MessageSource::User { id, .. }
            | cloto_shared::MessageSource::Agent { id } => id.as_str(),
            cloto_shared::MessageSource::System => "system",
        }
    }

    #[allow(clippy::too_many_lines)]
    pub async fn handle_message(&self, msg: ClotoMessage) -> anyhow::Result<()> {
        let target_agent_id = msg
            .target_agent
            .clone()
            .or_else(|| msg.metadata.get("target_agent_id").cloned())
            .unwrap_or_else(|| self.default_agent_id.clone());

        // Set active cron context if this message was dispatched by a cron job
        let cron_context_key = if let Some(cron_job_id) = msg.metadata.get("cron_job_id") {
            let generation = crate::db::get_cron_job_generation(&self.pool, cron_job_id)
                .await
                .unwrap_or(0);
            self.active_cron_contexts.insert(
                target_agent_id.clone(),
                crate::CronExecContext {
                    job_id: cron_job_id.clone(),
                    generation,
                },
            );
            Some(target_agent_id.clone())
        } else {
            None
        };

        // 1. エージェント情報の取得
        let (agent, default_engine_id) = self
            .agent_manager
            .get_agent_config(&target_agent_id)
            .await?;

        // Block disabled agents from processing messages
        if !agent.enabled {
            info!(agent_id = %target_agent_id, "🔌 Agent is powered off. Message dropped.");
            return Ok(());
        }

        // Passive heartbeat: update last_seen on message routing
        if let Err(e) = self.agent_manager.touch_last_seen(&target_agent_id).await {
            warn!(agent_id = %target_agent_id, error = %e, "Failed to update last_seen");
        }

        // Persist user message to chat history (backend-side persistence)
        let now_ms = chrono::Utc::now().timestamp_millis();
        let skip_user_persist = msg
            .metadata
            .get("skip_user_persist")
            .is_some_and(|v| v == "true");

        if !skip_user_persist {
            let parent_id = msg.metadata.get("parent_id").cloned();
            let branch_index = msg
                .metadata
                .get("branch_index")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);

            let user_chat_msg = crate::db::ChatMessageRow {
                id: msg.id.clone(),
                agent_id: target_agent_id.clone(),
                user_id: Self::extract_user_id(&msg).to_string(),
                source: "user".to_string(),
                content: serde_json::to_string(
                    &serde_json::json!([{"type": "text", "text": &msg.content}]),
                )
                .unwrap_or_default(),
                metadata: None,
                created_at: now_ms,
                parent_id,
                branch_index,
            };
            if let Err(e) = crate::db::save_chat_message_reliable(&self.pool, &user_chat_msg).await
            {
                error!("Chat persist DROPPED user message: {}", e);
            }
        }

        // 1-B. Image Analysis: analyze attached images before routing to engine
        let msg = self.maybe_analyze_images(msg).await;

        // 2. メモリからのコンテキスト取得 (Dual Dispatch: Rust Plugin → MCP Server)
        let memory_plugin = if let Some(preferred_id) = agent.metadata.get("preferred_memory") {
            self.registry.get_engine(preferred_id).await
        } else {
            self.registry.find_memory().await
        };

        // MCP fallback: find MCP server with store+recall tools
        // 🔐 Only use memory server if agent has access to it (checked via mcp_access_control)
        let granted_server_ids: Vec<String> = self
            .agent_manager
            .get_granted_server_ids(&target_agent_id)
            .await
            .unwrap_or_default();

        let mcp_memory: Option<(Arc<McpClientManager>, String)> = if memory_plugin.is_none() {
            if let Some(ref mcp) = self.registry.mcp_manager {
                mcp.resolve_capability_server(crate::managers::CapabilityType::Memory).await.and_then(|server_id| {
                    if granted_server_ids.contains(&server_id) {
                        Some((mcp.clone(), server_id))
                    } else {
                        tracing::info!(
                            agent_id = %target_agent_id,
                            server_id = %server_id,
                            "🔐 Agent lacks access to memory server — memory skipped"
                        );
                        None
                    }
                })
            } else {
                None
            }
        } else {
            None
        };

        let context = if let Some(ref plugin) = memory_plugin {
            if let Some(mem) = plugin.as_memory() {
                // 🔐 Check MemoryRead permission before recall
                let manifest = plugin.manifest();
                let reg_state = self.registry.state.read().await;
                let plugin_cloto_id = cloto_shared::ClotoId::from_name(&manifest.id);
                let has_memory_read = reg_state.effective_permissions
                    .get(&plugin_cloto_id)
                    .is_some_and(|p| p.contains(&cloto_shared::Permission::MemoryRead));
                drop(reg_state);
                if has_memory_read {
                    // 🛑 停滞対策: メモリの呼び出しにタイムアウトを設定
                    match tokio::time::timeout(
                        Duration::from_secs(self.memory_timeout_secs),
                        mem.recall(agent.id.clone(), &msg.content, self.memory_context_limit),
                    )
                    .await
                    {
                        Ok(Ok(ctx)) => ctx,
                        Ok(Err(e)) => {
                            error!(agent_id = %agent.id, error = %e, "❌ Memory recall failed");
                            vec![]
                        }
                        Err(_) => {
                            error!(agent_id = %agent.id, "⏱️ Memory recall timed out");
                            vec![]
                        }
                    }
                } else {
                    tracing::warn!(
                        plugin_id = %manifest.id,
                        "⚠️  Memory plugin lacks MemoryRead permission — context recall skipped"
                    );
                    vec![]
                } // end has_memory_read else branch
            } else {
                vec![]
            }
        } else if let Some((ref mcp, ref server_id)) = mcp_memory {
            // MCP Memory Resolver: recall via MCP server
            let recall_args = serde_json::json!({
                "agent_id": agent.id,
                "query": msg.content,
                "limit": self.memory_context_limit,
            });
            match tokio::time::timeout(
                Duration::from_secs(self.memory_timeout_secs),
                mcp.call_server_tool(server_id, "recall", recall_args),
            )
            .await
            {
                Ok(Ok(result)) => Self::parse_mcp_recall_result(&result),
                Ok(Err(e)) => {
                    error!(agent_id = %agent.id, server_id = %server_id, error = %e, "❌ MCP memory recall failed");
                    vec![]
                }
                Err(_) => {
                    error!(agent_id = %agent.id, server_id = %server_id, "⏱️ MCP memory recall timed out");
                    vec![]
                }
            }
        } else {
            vec![]
        };

        // 3. 【核心】思考要求イベントを発行
        info!(
            target_agent_id = %target_agent_id,
            agent_name = %agent.name,
            engine_id = %default_engine_id,
            "📢 Dispatching Thought/Consensus Request"
        );

        let trace_id = cloto_shared::ClotoId::new_trace_id();

        if msg.content.to_lowercase().starts_with("consensus:") {
            // 合意形成モード
            let thought_event_data = cloto_shared::ClotoEventData::ConsensusRequested {
                task: msg.content.clone(),
                engine_ids: self.consensus_engines.clone(),
            };

            let envelope = crate::EnvelopedEvent {
                event: Arc::new(cloto_shared::ClotoEvent::with_trace(
                    trace_id,
                    thought_event_data,
                )),
                issuer: None,
                correlation_id: None,
                depth: 0,
            };
            if let Err(e) = self.sender.send(envelope).await {
                error!("Failed to dispatch ConsensusRequested: {}", e);
            }

            // 各エンジンにも個別にThoughtRequestedを投げる (Moderatorが拾うため)
            for engine in &self.consensus_engines {
                let inner_thought = cloto_shared::ClotoEventData::ThoughtRequested {
                    agent: agent.clone(),
                    engine_id: engine.clone(),
                    message: msg.clone(),
                    context: context.clone(),
                };
                let env = crate::EnvelopedEvent {
                    event: Arc::new(cloto_shared::ClotoEvent::with_trace(
                        trace_id,
                        inner_thought,
                    )),
                    issuer: None,
                    correlation_id: Some(trace_id),
                    depth: 1,
                };
                if let Err(e) = self.sender.send(env).await {
                    error!(
                        "Failed to dispatch ThoughtRequested for engine {}: {}",
                        engine, e
                    );
                }
            }
        } else {
            // 通常モード: エージェントループで処理
            // 3-layer engine selection: override > routing rules > default
            let selection = if let Some(ov) = msg.metadata.get("engine_override") {
                EngineSelection {
                    engine_id: ov.clone(),
                    cfr: false,
                    escalate_to: None,
                    fallback: None,
                }
            } else if let Some(ref mcp) = self.registry.mcp_manager {
                let connected = mcp.list_connected_mind_servers().await;
                evaluate_engine_routing(
                    &msg.content,
                    &agent.metadata,
                    &connected,
                    &default_engine_id,
                )
            } else {
                evaluate_engine_routing(&msg.content, &agent.metadata, &[], &default_engine_id)
            };

            let engine_id = selection.engine_id.clone();

            // Execute with CFR + fallback support
            let (final_result, final_engine_id) = if selection.cfr {
                // CFR Tier 1: tool-less think only (judgment mode)
                let tier1_result = {
                    let engine_plugin = self.registry.get_engine(&engine_id).await;
                    let mcp_engine = if engine_plugin.is_none() {
                        self.registry.mcp_manager.clone()
                    } else {
                        None
                    };
                    self.engine_think(
                        engine_plugin.as_ref(),
                        mcp_engine.as_ref(),
                        &engine_id,
                        &agent,
                        &msg,
                        context.clone(),
                    )
                    .await
                };

                match tier1_result {
                    Ok(content) if needs_escalation(&content) => {
                        // Tier 1 requested escalation → Tier 2 with full agentic loop
                        let escalate_id = selection.escalate_to.as_deref().unwrap_or(&engine_id);
                        info!(from = %engine_id, to = %escalate_id, "⬆️ CFR escalation triggered");
                        let r = self
                            .run_agentic_loop(
                                &agent,
                                escalate_id,
                                &msg,
                                context.clone(),
                                &granted_server_ids,
                                trace_id,
                            )
                            .await;
                        (r, escalate_id.to_string())
                    }
                    Ok(content) => {
                        // Tier 1 handled it directly
                        info!(engine = %engine_id, "⚡ CFR Tier 1 handled request");
                        (Ok(content), engine_id.clone())
                    }
                    Err(e) if is_retriable_error(&e) && selection.fallback.is_some() => {
                        let fallback_id = selection.fallback.as_deref().unwrap();
                        warn!(from = %engine_id, to = %fallback_id, error = %e, "🔄 CFR Tier 1 fallback");
                        let r = self
                            .run_agentic_loop(
                                &agent,
                                fallback_id,
                                &msg,
                                context.clone(),
                                &granted_server_ids,
                                trace_id,
                            )
                            .await;
                        (r, fallback_id.to_string())
                    }
                    Err(e) => (Err(e), engine_id.clone()),
                }
            } else {
                // Standard execution (no CFR)
                let loop_result = self
                    .run_agentic_loop(
                        &agent,
                        &engine_id,
                        &msg,
                        context.clone(),
                        &granted_server_ids,
                        trace_id,
                    )
                    .await;

                match loop_result {
                    Ok(content) => (Ok(content), engine_id.clone()),
                    Err(e) if is_retriable_error(&e) && selection.fallback.is_some() => {
                        let fallback_id = selection.fallback.as_deref().unwrap();
                        warn!(from = %engine_id, to = %fallback_id, error = %e, "🔄 Auto-fallback triggered");
                        let r = self
                            .run_agentic_loop(
                                &agent,
                                fallback_id,
                                &msg,
                                context.clone(),
                                &granted_server_ids,
                                trace_id,
                            )
                            .await;
                        (r, fallback_id.to_string())
                    }
                    Err(e) => (Err(e), engine_id.clone()),
                }
            };

            let engine_id = final_engine_id;
            match final_result {
                Ok(content) => {
                    // エージェント返答もメモリに保存 (user messageと対で保存)
                    if let Some(plugin) = &memory_plugin {
                        let plugin_clone = plugin.clone();
                        let agent_resp_msg = ClotoMessage {
                            id: format!("{}-resp", msg.id),
                            source: cloto_shared::MessageSource::Agent {
                                id: agent.id.clone(),
                            },
                            target_agent: Some(agent.id.clone()),
                            content: content.clone(),
                            timestamp: Utc::now(),
                            metadata: std::collections::HashMap::new(),
                        };
                        let agent_id_clone = agent.id.clone();
                        let mem_timeout = Duration::from_secs(self.memory_timeout_secs);
                        tokio::spawn(async move {
                            if let Some(mem) = plugin_clone.as_memory() {
                                let _ = tokio::time::timeout(
                                    mem_timeout,
                                    mem.store(agent_id_clone, agent_resp_msg),
                                )
                                .await;
                            }
                        });
                    } else if let Some((ref mcp, ref server_id)) = mcp_memory {
                        let mcp_clone = mcp.clone();
                        let server_id_clone = server_id.clone();
                        let agent_id_clone = agent.id.clone();
                        let resp_msg_json = serde_json::json!({
                            "id": format!("{}-resp", msg.id),
                            "content": content.clone(),
                            "source": { "type": "Agent", "id": agent.id },
                            "timestamp": Utc::now().to_rfc3339(),
                        });
                        let mem_timeout2 = Duration::from_secs(self.memory_timeout_secs);
                        tokio::spawn(async move {
                            let store_args = serde_json::json!({
                                "agent_id": agent_id_clone,
                                "message": resp_msg_json,
                            });
                            let _ = tokio::time::timeout(
                                mem_timeout2,
                                mcp_clone.call_server_tool(&server_id_clone, "store", store_args),
                            )
                            .await;
                        });
                    }

                    // Persist agent response to chat history (backend-side)
                    let resp_id = format!("{}-resp", msg.id);
                    // For retry: metadata["parent_id"] overrides default parent (the user msg ID)
                    let response_parent = msg
                        .metadata
                        .get("parent_id")
                        .cloned()
                        .unwrap_or_else(|| msg.id.clone());
                    let resp_branch =
                        crate::db::get_next_branch_index(&self.pool, &response_parent)
                            .await
                            .unwrap_or(0);
                    let agent_chat_msg = crate::db::ChatMessageRow {
                        id: resp_id,
                        agent_id: agent.id.clone(),
                        user_id: Self::extract_user_id(&msg).to_string(),
                        source: "agent".to_string(),
                        content: serde_json::to_string(
                            &serde_json::json!([{"type": "text", "text": &content}]),
                        )
                        .unwrap_or_default(),
                        metadata: None,
                        created_at: chrono::Utc::now().timestamp_millis(),
                        parent_id: Some(response_parent),
                        branch_index: resp_branch,
                    };
                    if let Err(e) =
                        crate::db::save_chat_message_reliable(&self.pool, &agent_chat_msg).await
                    {
                        error!("Chat persist DROPPED agent response: {}", e);
                    }

                    let thought_response = ClotoEventData::ThoughtResponse {
                        agent_id: agent.id.clone(),
                        engine_id: engine_id.clone(),
                        content,
                        source_message_id: msg.id.clone(),
                    };
                    let envelope = crate::EnvelopedEvent {
                        event: Arc::new(ClotoEvent::with_trace(trace_id, thought_response)),
                        issuer: None,
                        correlation_id: None,
                        depth: 0,
                    };
                    if let Err(e) = self.sender.send(envelope).await {
                        error!(
                            target_agent_id = %target_agent_id,
                            error = %e,
                            "❌ Failed to send ThoughtResponse"
                        );
                    }
                }
                Err(e) => {
                    error!(
                        agent_id = %agent.id,
                        engine_id = %engine_id,
                        error = %e,
                        "❌ Agentic loop failed"
                    );
                    // H-04: Send error response so the user's message doesn't vanish
                    let error_content = format!("[Error] {}", e);

                    // Persist error response to chat history
                    let err_resp_id = format!("{}-resp", msg.id);
                    let err_response_parent = msg
                        .metadata
                        .get("parent_id")
                        .cloned()
                        .unwrap_or_else(|| msg.id.clone());
                    let err_resp_branch =
                        crate::db::get_next_branch_index(&self.pool, &err_response_parent)
                            .await
                            .unwrap_or(0);
                    let err_chat_msg = crate::db::ChatMessageRow {
                        id: err_resp_id,
                        agent_id: agent.id.clone(),
                        user_id: Self::extract_user_id(&msg).to_string(),
                        source: "agent".to_string(),
                        content: serde_json::to_string(
                            &serde_json::json!([{"type": "text", "text": &error_content}]),
                        )
                        .unwrap_or_default(),
                        metadata: None,
                        created_at: chrono::Utc::now().timestamp_millis(),
                        parent_id: Some(err_response_parent),
                        branch_index: err_resp_branch,
                    };
                    if let Err(e) =
                        crate::db::save_chat_message_reliable(&self.pool, &err_chat_msg).await
                    {
                        error!("Chat persist DROPPED error response: {}", e);
                    }

                    let error_response = ClotoEventData::ThoughtResponse {
                        agent_id: agent.id.clone(),
                        engine_id: engine_id.clone(),
                        content: error_content,
                        source_message_id: msg.id.clone(),
                    };
                    let envelope = crate::EnvelopedEvent {
                        event: Arc::new(ClotoEvent::with_trace(trace_id, error_response)),
                        issuer: None,
                        correlation_id: None,
                        depth: 0,
                    };
                    let _ = self.sender.send(envelope).await;
                }
            }
        }

        // メモリへの保存 (below agentic loop / consensus dispatch)
        if let Some(plugin) = memory_plugin {
            if let Some(_mem) = plugin.as_memory() {
                // 🔐 Check MemoryWrite permission before store
                let manifest = plugin.manifest();
                let has_memory_write = {
                    let reg_state = self.registry.state.read().await;
                    let pid = cloto_shared::ClotoId::from_name(&manifest.id);
                    reg_state.effective_permissions
                        .get(&pid)
                        .is_some_and(|p| p.contains(&cloto_shared::Permission::MemoryWrite))
                };
                if has_memory_write {
                    let agent_id = agent.id.clone();
                    let plugin_clone = plugin.clone();
                    let metrics = self.metrics.clone();
                    let store_mem_timeout = Duration::from_secs(self.memory_timeout_secs);
                    // 🛑 停滞対策: 保存処理はバックグラウンドで行い、メインループをブロックしない
                    tokio::spawn(async move {
                        if let Some(mem) = plugin_clone.as_memory() {
                            match tokio::time::timeout(
                                store_mem_timeout,
                                mem.store(agent_id.clone(), msg),
                            )
                            .await
                            {
                                Ok(Ok(())) => {
                                    metrics
                                        .total_memories
                                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                }
                                Ok(Err(e)) => {
                                    error!(agent_id = %agent_id, error = %e, "❌ Memory store failed");
                                }
                                Err(_) => {
                                    error!(agent_id = %agent_id, "❌ Memory store timed out (5s)");
                                }
                            }
                        }
                    });
                } else {
                    tracing::warn!(
                        plugin_id = %manifest.id,
                        "⚠️  Memory plugin lacks MemoryWrite permission — store skipped"
                    );
                } // end has_memory_write branch
            }
        } else if let Some((mcp, server_id)) = mcp_memory {
            // MCP Memory Store: store user message via MCP server
            let agent_id = agent.id.clone();
            let metrics = self.metrics.clone();
            let msg_json = serde_json::json!({
                "id": msg.id,
                "content": msg.content,
                "source": { "type": "User", "id": "", "name": "" },
                "timestamp": msg.timestamp.to_rfc3339(),
            });

            // Clone for episode archival (before mcp/server_id are moved)
            let ep_mcp = mcp.clone();
            let ep_server_id = server_id.clone();
            let ep_agent_id = agent_id.clone();
            let memory_timeout = Duration::from_secs(self.memory_timeout_secs);

            tokio::spawn(async move {
                let store_args = serde_json::json!({
                    "agent_id": agent_id,
                    "message": msg_json,
                });
                match tokio::time::timeout(
                    memory_timeout,
                    mcp.call_server_tool(&server_id, "store", store_args),
                )
                .await
                {
                    Ok(Ok(_)) => {
                        metrics
                            .total_memories
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    Ok(Err(e)) => {
                        error!(agent_id = %agent_id, error = %e, "❌ MCP memory store failed");
                    }
                    Err(_) => {
                        error!(agent_id = %agent_id, "❌ MCP memory store timed out");
                    }
                }
            });

            // Episode auto-archival check (background, non-blocking)
            let ep_memory_timeout = Duration::from_secs(self.memory_timeout_secs);
            tokio::spawn(async move {
                Self::maybe_archive_episode(
                    &ep_mcp,
                    &ep_server_id,
                    &ep_agent_id,
                    ep_memory_timeout,
                )
                .await;
            });
        }

        // Clear active cron context
        if let Some(ref agent_key) = cron_context_key {
            self.active_cron_contexts.remove(agent_key);
        }

        Ok(())
    }

    // ── Agentic Loop ──

    #[allow(clippy::too_many_lines)]
    async fn run_agentic_loop(
        &self,
        agent: &AgentMetadata,
        engine_id: &str,
        message: &ClotoMessage,
        context: Vec<ClotoMessage>,
        agent_plugin_ids: &[String],
        trace_id: ClotoId,
    ) -> anyhow::Result<String> {
        // Engine Resolver: try Rust plugin first, then fall back to MCP server
        let engine_plugin = self.registry.get_engine(engine_id).await;
        let mcp_engine = if engine_plugin.is_none() {
            // Check if an MCP server with this engine ID exists
            if let Some(ref mcp) = self.registry.mcp_manager {
                if mcp.has_server(engine_id).await {
                    Some(mcp.clone())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        if engine_plugin.is_none() && mcp_engine.is_none() {
            return Err(anyhow::anyhow!("Engine '{}' not found", engine_id));
        }

        // Determine tool support
        let supports_tools = if let Some(ref plugin) = engine_plugin {
            plugin
                .as_reasoning()
                .is_some_and(cloto_shared::ReasoningEngine::supports_tools)
        } else if let Some(ref mcp) = mcp_engine {
            // MCP engine supports tools if it has a 'think_with_tools' tool
            mcp.has_server_tool(engine_id, "think_with_tools").await
        } else {
            false
        };

        // Fallback: engine does not support tools → plain think()
        if !supports_tools {
            return self
                .engine_think(
                    engine_plugin.as_ref(),
                    mcp_engine.as_ref(),
                    engine_id,
                    agent,
                    message,
                    context,
                )
                .await;
        }

        // エージェントに割り当てられたプラグインのみからツールを収集
        let tools = if agent_plugin_ids.is_empty() {
            self.registry.collect_tool_schemas().await
        } else {
            self.registry
                .collect_tool_schemas_for_agent(agent_plugin_ids, &agent.id)
                .await
        };
        if tools.is_empty() {
            return self
                .engine_think(
                    engine_plugin.as_ref(),
                    mcp_engine.as_ref(),
                    engine_id,
                    agent,
                    message,
                    context,
                )
                .await;
        }

        // M-04: Build tool name set for pre-validation (avoid timeout waiting for non-existent tools)
        let tool_names: std::collections::HashSet<String> = tools
            .iter()
            .filter_map(|t| {
                t.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .map(std::string::ToString::to_string)
            })
            .collect();

        info!(
            agent_id = %agent.id,
            engine_id = %engine_id,
            tool_count = tools.len(),
            "🔄 Starting agentic loop"
        );

        let mut tool_history: Vec<serde_json::Value> = Vec::new();
        let mut iteration: u8 = 0;
        let mut total_tool_calls: u32 = 0;

        loop {
            iteration += 1;
            if iteration > self.max_agentic_iterations {
                warn!(
                    agent_id = %agent.id,
                    "⚠️ Agentic loop hit max iterations ({}), forcing text response",
                    self.max_agentic_iterations
                );
                return self
                    .engine_think(
                        engine_plugin.as_ref(),
                        mcp_engine.as_ref(),
                        engine_id,
                        agent,
                        message,
                        context.clone(),
                    )
                    .await;
            }

            let result = self
                .engine_think_with_tools(
                    engine_plugin.as_ref(),
                    mcp_engine.as_ref(),
                    engine_id,
                    agent,
                    message,
                    context.clone(),
                    &tools,
                    &tool_history,
                )
                .await?;

            match result {
                ThinkResult::Final(content) => {
                    // Emit loop completion event
                    self.emit_event(
                        trace_id,
                        ClotoEventData::AgenticLoopCompleted {
                            agent_id: agent.id.clone(),
                            engine_id: engine_id.to_string(),
                            total_iterations: iteration,
                            total_tool_calls,
                            source_message_id: message.id.clone(),
                        },
                    )
                    .await;

                    info!(
                        agent_id = %agent.id,
                        iterations = iteration,
                        tool_calls = total_tool_calls,
                        "✅ Agentic loop completed"
                    );
                    return Ok(content);
                }
                ThinkResult::ToolCalls {
                    assistant_content,
                    calls,
                } => {
                    info!(
                        agent_id = %agent.id,
                        iteration = iteration,
                        num_calls = calls.len(),
                        "🔧 LLM requested tool calls"
                    );

                    // Build assistant message with tool_calls for history
                    let tool_calls_json: Vec<serde_json::Value> = calls
                        .iter()
                        .map(|tc| {
                            serde_json::json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": tc.arguments.to_string()
                                }
                            })
                        })
                        .collect();

                    let mut assistant_msg = serde_json::json!({
                        "role": "assistant",
                        "tool_calls": tool_calls_json
                    });
                    if let Some(ref content) = assistant_content {
                        assistant_msg["content"] = serde_json::json!(content);
                        // Emit thinking event so the frontend can show intermediate reasoning
                        if !content.is_empty() {
                            self.emit_event(
                                trace_id,
                                ClotoEventData::AgentThinking {
                                    agent_id: agent.id.clone(),
                                    engine_id: engine_id.to_string(),
                                    content: content.clone(),
                                    iteration,
                                },
                            )
                            .await;
                        }
                    }
                    tool_history.push(assistant_msg);

                    // ── Batch Command Approval Gate ──
                    let yolo =
                        self.registry.mcp_manager.as_ref().is_some_and(|m| {
                            m.yolo_mode.load(std::sync::atomic::Ordering::Relaxed)
                        });
                    let denied_call_ids = command_approval::run_approval_gate(
                        &calls,
                        &agent.id,
                        trace_id,
                        yolo,
                        self.registry.mcp_manager.as_ref(),
                        &self.pending_approvals,
                        &self.session_trusted_commands,
                        &self.pool,
                        &self.sender,
                    )
                    .await;

                    // Execute each tool call
                    for call in &calls {
                        total_tool_calls += 1;

                        // Skip denied commands
                        if denied_call_ids.contains(&call.id) {
                            let cmd = call
                                .arguments
                                .get("command")
                                .and_then(|v| v.as_str())
                                .unwrap_or("?");
                            tool_history.push(serde_json::json!({
                                "role": "tool",
                                "tool_call_id": call.id,
                                "content": format!("Error: command '{}' was denied by user", cmd)
                            }));
                            continue;
                        }

                        // M-04: Pre-validate tool name before execution
                        if !tool_names.contains(&call.name) {
                            warn!(
                                tool = %call.name,
                                "⚠️ LLM requested non-existent tool, skipping"
                            );
                            tool_history.push(serde_json::json!({
                                "role": "tool",
                                "tool_call_id": call.id,
                                "content": format!("Error: tool '{}' not found", call.name)
                            }));
                            continue;
                        }

                        let start = std::time::Instant::now();

                        // 🔐 Anti-spoofing: always force agent_id in tool arguments
                        let mut safe_args = call.arguments.clone();
                        if let Some(obj) = safe_args.as_object_mut() {
                            obj.insert(
                                "agent_id".to_string(),
                                serde_json::Value::String(agent.id.clone()),
                            );
                        }

                        let tool_result = tokio::time::timeout(
                            Duration::from_secs(self.tool_execution_timeout_secs),
                            async {
                                if agent_plugin_ids.is_empty() {
                                    self.registry.execute_tool(&call.name, safe_args).await
                                } else {
                                    self.registry
                                        .execute_tool_for_agent(
                                            agent_plugin_ids,
                                            &agent.id,
                                            &call.name,
                                            safe_args,
                                        )
                                        .await
                                }
                            },
                        )
                        .await;

                        let duration_ms = start.elapsed().as_millis() as u64;

                        let (success, content) = match tool_result {
                            Ok(Ok(v)) => (true, v.to_string()),
                            Ok(Err(e)) => (false, format!("Error: {}", e)),
                            Err(_) => (false, "Error: tool execution timed out".to_string()),
                        };

                        info!(
                            tool = %call.name,
                            success = success,
                            duration_ms = duration_ms,
                            "  🔧 Tool executed"
                        );

                        // Emit observability event
                        self.emit_event(
                            trace_id,
                            ClotoEventData::ToolInvoked {
                                agent_id: agent.id.clone(),
                                engine_id: engine_id.to_string(),
                                tool_name: call.name.clone(),
                                call_id: call.id.clone(),
                                success,
                                duration_ms,
                                iteration,
                            },
                        )
                        .await;

                        // Add tool result to history (OpenAI format)
                        tool_history.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": call.id,
                            "content": content
                        }));
                    }

                    // M-03: Prevent unbounded tool_history growth
                    if tool_history.len() > MAX_TOOL_HISTORY {
                        let excess = tool_history.len() - MAX_TOOL_HISTORY;
                        tool_history.drain(..excess);
                    }
                }
            }
        }
    }

    // ── Engine Dispatch Helpers (Rust Plugin / MCP Dual Dispatch) ──

    /// Call engine's think() — routes to either Rust plugin or MCP server.
    async fn engine_think(
        &self,
        engine_plugin: Option<&Arc<dyn Plugin>>,
        mcp_engine: Option<&Arc<McpClientManager>>,
        engine_id: &str,
        agent: &AgentMetadata,
        message: &ClotoMessage,
        context: Vec<ClotoMessage>,
    ) -> anyhow::Result<String> {
        if let Some(plugin) = engine_plugin {
            let engine = plugin.as_reasoning().ok_or_else(|| {
                anyhow::anyhow!("Plugin '{}' is not a ReasoningEngine", engine_id)
            })?;
            return engine.think(agent, message, context).await;
        }

        if let Some(mcp) = mcp_engine {
            let args = serde_json::json!({
                "agent": serde_json::to_value(agent)?,
                "message": serde_json::to_value(message)?,
                "context": context.iter().map(|m| {
                    serde_json::json!({
                        "source": m.source,
                        "content": m.content,
                    })
                }).collect::<Vec<_>>(),
            });
            let result = mcp.call_server_tool(engine_id, "think", args).await?;
            return Self::extract_mcp_think_content(&result);
        }

        Err(anyhow::anyhow!("Engine '{}' not found", engine_id))
    }

    /// Call engine's think_with_tools() — routes to either Rust plugin or MCP server.
    async fn engine_think_with_tools(
        &self,
        engine_plugin: Option<&Arc<dyn Plugin>>,
        mcp_engine: Option<&Arc<McpClientManager>>,
        engine_id: &str,
        agent: &AgentMetadata,
        message: &ClotoMessage,
        context: Vec<ClotoMessage>,
        tools: &[serde_json::Value],
        tool_history: &[serde_json::Value],
    ) -> anyhow::Result<ThinkResult> {
        if let Some(plugin) = engine_plugin {
            let engine = plugin.as_reasoning().ok_or_else(|| {
                anyhow::anyhow!("Plugin '{}' is not a ReasoningEngine", engine_id)
            })?;
            return engine
                .think_with_tools(agent, message, context, tools, tool_history)
                .await;
        }

        if let Some(mcp) = mcp_engine {
            let args = serde_json::json!({
                "agent": serde_json::to_value(agent)?,
                "message": serde_json::to_value(message)?,
                "context": context.iter().map(|m| {
                    serde_json::json!({
                        "source": m.source,
                        "content": m.content,
                    })
                }).collect::<Vec<_>>(),
                "tools": tools,
                "tool_history": tool_history,
            });
            let result = mcp
                .call_server_tool(engine_id, "think_with_tools", args)
                .await?;
            return Self::parse_mcp_think_result(&result);
        }

        Err(anyhow::anyhow!("Engine '{}' not found", engine_id))
    }

    /// Extract text content from MCP think() response.
    /// Analyze image attachments via vision.capture MCP server.
    /// Prepends analysis text to the message content so the LLM engine
    /// can "see" images even though it only receives text.
    async fn maybe_analyze_images(
        &self,
        mut msg: cloto_shared::ClotoMessage,
    ) -> cloto_shared::ClotoMessage {
        // Fetch attachments from chat persistence DB
        let Ok(attachments) =
            crate::db::get_attachments_for_message(&self.agent_manager.pool, &msg.id).await
        else {
            return msg;
        };

        let image_atts: Vec<_> = attachments
            .iter()
            .filter(|a| a.mime_type.starts_with("image/"))
            .collect();

        if image_atts.is_empty() {
            return msg;
        }

        let Some(ref mcp) = self.registry.mcp_manager else {
            return msg;
        };

        let mut analyses = Vec::new();
        for att in &image_atts {
            // Get image bytes
            let image_bytes = if let Some(ref data) = att.inline_data {
                data.clone()
            } else if let Some(ref path) = att.disk_path {
                match tokio::fs::read(path).await {
                    Ok(d) => d,
                    Err(_) => continue,
                }
            } else {
                continue;
            };

            // Write to temp file for vision MCP
            let ext = att.mime_type.strip_prefix("image/").unwrap_or("png");
            let ext = if ext == "jpeg" { "jpg" } else { ext };
            let temp_path = format!("data/tmp_vision_{}.{ext}", uuid::Uuid::new_v4());
            if tokio::fs::write(&temp_path, &image_bytes).await.is_err() {
                continue;
            }

            let abs_path = match std::path::Path::new(&temp_path).canonicalize() {
                Ok(p) => p.to_string_lossy().to_string(),
                Err(_) => temp_path.clone(),
            };

            let args = serde_json::json!({
                "file_path": abs_path,
                "prompt": format!(
                    "Analyze this image. The user said: '{}'. Describe what you see in detail.",
                    msg.content
                )
            });

            match mcp
                .call_capability_tool(crate::managers::CapabilityType::Vision, "analyze_image", args, None)
                .await
            {
                Ok(result) => {
                    if let Ok(text) = Self::extract_mcp_think_content(&result) {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                            if let Some(response) = json.get("response").and_then(|r| r.as_str()) {
                                analyses.push(response.to_string());
                            }
                        } else {
                            analyses.push(text);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        msg_id = %msg.id,
                        "Image analysis failed, continuing without vision context"
                    );
                }
            }

            // Clean up temp file
            let _ = tokio::fs::remove_file(&temp_path).await;
        }

        if !analyses.is_empty() {
            msg.content = format!(
                "[Image Analysis]\n{}\n\n[User Message]\n{}",
                analyses.join("\n---\n"),
                msg.content
            );
            tracing::info!(
                msg_id = %msg.id,
                image_count = image_atts.len(),
                "Prepended vision analysis to message content"
            );
        }

        msg
    }

    /// Build a user-friendly error message from MCP engine error + optional error_code.
    fn format_engine_error(error: &str, error_code: Option<&str>) -> String {
        let guidance = match error_code.unwrap_or("unknown") {
            "auth_failed" => " Check your API key in Settings → Security.",
            "rate_limited" => " Please wait a moment and try again.",
            "provider_error" => " The LLM provider is experiencing issues. Try a different engine.",
            "connection_failed" | "timeout" => " Ensure the kernel and LLM services are running.",
            _ => "",
        };
        format!("{error}{guidance}")
    }

    fn extract_mcp_think_content(
        result: &crate::managers::mcp_protocol::CallToolResult,
    ) -> anyhow::Result<String> {
        use crate::managers::mcp_protocol::ToolContent;
        for content in &result.content {
            if let ToolContent::Text { text } = content {
                // Try to parse as JSON (may contain {"type":"final","content":"..."})
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(text) {
                    if let Some(error) = json.get("error").and_then(|e| e.as_str()) {
                        let code = json.get("error_code").and_then(|c| c.as_str());
                        return Err(anyhow::anyhow!(
                            "{}",
                            Self::format_engine_error(error, code)
                        ));
                    }
                    if let Some(content) = json.get("content").and_then(|c| c.as_str()) {
                        return Ok(content.to_string());
                    }
                }
                // Fall back to raw text
                return Ok(text.clone());
            }
        }
        Err(anyhow::anyhow!("MCP engine returned no text content"))
    }

    /// Parse ThinkResult from MCP think_with_tools() response.
    fn parse_mcp_think_result(
        result: &crate::managers::mcp_protocol::CallToolResult,
    ) -> anyhow::Result<ThinkResult> {
        use crate::managers::mcp_protocol::ToolContent;
        for content in &result.content {
            if let ToolContent::Text { text } = content {
                let json: serde_json::Value = serde_json::from_str(text)
                    .map_err(|e| anyhow::anyhow!("MCP engine returned invalid JSON: {}", e))?;

                if let Some(error) = json.get("error").and_then(|e| e.as_str()) {
                    let code = json.get("error_code").and_then(|c| c.as_str());
                    return Err(anyhow::anyhow!(
                        "{}",
                        Self::format_engine_error(error, code)
                    ));
                }

                let result_type = json.get("type").and_then(|t| t.as_str()).unwrap_or("final");

                if result_type == "tool_calls" {
                    let assistant_content = json
                        .get("assistant_content")
                        .and_then(|c| c.as_str())
                        .map(std::string::ToString::to_string);
                    let calls_json = json
                        .get("calls")
                        .and_then(|c| c.as_array())
                        .cloned()
                        .unwrap_or_default();

                    let calls: Vec<ToolCall> = calls_json
                        .iter()
                        .filter_map(|tc| {
                            let id = tc.get("id")?.as_str()?.to_string();
                            let name = tc.get("name")?.as_str()?.to_string();
                            let arguments = tc
                                .get("arguments")
                                .cloned()
                                .unwrap_or(serde_json::json!({}));
                            Some(ToolCall {
                                id,
                                name,
                                arguments,
                            })
                        })
                        .collect();

                    return Ok(ThinkResult::ToolCalls {
                        assistant_content,
                        calls,
                    });
                }
                let content = json
                    .get("content")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string();
                return Ok(ThinkResult::Final(content));
            }
        }
        Err(anyhow::anyhow!(
            "MCP engine returned no parseable ThinkResult"
        ))
    }

    /// Parse MCP recall() response into Vec<ClotoMessage>, sorted by timestamp ascending
    /// (oldest first — the chronological order LLM engines expect).
    fn parse_mcp_recall_result(
        result: &crate::managers::mcp_protocol::CallToolResult,
    ) -> Vec<ClotoMessage> {
        use crate::managers::mcp_protocol::ToolContent;
        for content in &result.content {
            if let ToolContent::Text { text } = content {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(text) {
                    if let Some(error) = json.get("error").and_then(|e| e.as_str()) {
                        error!("MCP memory recall error: {}", error);
                        return vec![];
                    }
                    if let Some(messages) = json.get("messages").and_then(|m| m.as_array()) {
                        let mut result: Vec<ClotoMessage> = messages
                            .iter()
                            .filter_map(|m| {
                                let content = m.get("content")?.as_str()?.to_string();
                                let source = if let Some(src) = m.get("source") {
                                    serde_json::from_value(src.clone())
                                        .unwrap_or(cloto_shared::MessageSource::System)
                                } else {
                                    cloto_shared::MessageSource::System
                                };
                                let timestamp = m
                                    .get("timestamp")
                                    .and_then(|t| t.as_str())
                                    .and_then(|t| {
                                        chrono::DateTime::parse_from_rfc3339(t)
                                            .ok()
                                            .map(|dt| dt.with_timezone(&chrono::Utc))
                                    })
                                    .unwrap_or_else(Utc::now);
                                let id = m
                                    .get("id")
                                    .and_then(|i| i.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                Some(ClotoMessage {
                                    id,
                                    source,
                                    target_agent: None,
                                    content,
                                    timestamp,
                                    metadata: std::collections::HashMap::new(),
                                })
                            })
                            .collect();
                        // G1.4: Sort by timestamp ascending (oldest→newest)
                        result.sort_by_key(|m| m.timestamp);
                        return result;
                    }
                }
            }
        }
        vec![]
    }

    /// Auto-archive episode when enough unarchived memories accumulate.
    async fn maybe_archive_episode(
        mcp: &Arc<McpClientManager>,
        server_id: &str,
        agent_id: &str,
        memory_timeout: Duration,
    ) {
        // 1. Fetch recent memories
        let Ok(Ok(mem_result)) = tokio::time::timeout(
            memory_timeout,
            mcp.call_server_tool(
                server_id,
                "list_memories",
                serde_json::json!({"agent_id": agent_id, "limit": TOOL_USAGE_THRESHOLD + 5}),
            ),
        )
        .await
        else {
            return;
        };

        let Some(mem_json) = Self::extract_tool_json(&mem_result) else {
            return;
        };
        let memories = match mem_json.get("memories").and_then(|m| m.as_array()) {
            Some(m) if m.len() >= TOOL_USAGE_THRESHOLD => m,
            _ => return,
        };

        // 2. Get last episode timestamp
        let Ok(Ok(ep_result)) = tokio::time::timeout(
            memory_timeout,
            mcp.call_server_tool(
                server_id,
                "list_episodes",
                serde_json::json!({"agent_id": agent_id, "limit": 1}),
            ),
        )
        .await
        else {
            return;
        };

        let last_ep_time = Self::extract_tool_json(&ep_result).and_then(|j| {
            j.get("episodes")?
                .as_array()?
                .first()?
                .get("created_at")?
                .as_str()
                .map(String::from)
        });

        // 3. Count unarchived memories
        let unarchived: Vec<&serde_json::Value> = if let Some(ref ep_time) = last_ep_time {
            memories
                .iter()
                .filter(|m| {
                    m.get("created_at").and_then(|t| t.as_str()).unwrap_or("") > ep_time.as_str()
                })
                .collect()
        } else {
            memories.iter().collect()
        };

        if unarchived.len() < TOOL_USAGE_THRESHOLD {
            return;
        }

        // 4. Archive
        let history: Vec<serde_json::Value> = unarchived
            .iter()
            .map(|m| {
                serde_json::json!({
                    "content": m.get("content"),
                    "source": m.get("source"),
                    "timestamp": m.get("timestamp"),
                })
            })
            .collect();

        match mcp
            .call_server_tool(
                server_id,
                "archive_episode",
                serde_json::json!({"agent_id": agent_id, "history": history}),
            )
            .await
        {
            Ok(_) => {
                info!(
                    agent_id = %agent_id,
                    message_count = unarchived.len(),
                    "📚 Auto-archived episode"
                );

                // Also update user profile after successful archival
                match mcp
                    .call_server_tool(
                        server_id,
                        "update_profile",
                        serde_json::json!({"agent_id": agent_id, "history": history}),
                    )
                    .await
                {
                    Ok(_) => {
                        info!(agent_id = %agent_id, "📝 Auto-updated user profile");
                    }
                    Err(e) => {
                        warn!(agent_id = %agent_id, error = %e, "⚠️ Profile update failed");
                    }
                }
            }
            Err(e) => {
                warn!(agent_id = %agent_id, error = %e, "⚠️ Episode archival failed");
            }
        }
    }

    /// Extract JSON from an MCP CallToolResult's first text content.
    fn extract_tool_json(
        result: &crate::managers::mcp_protocol::CallToolResult,
    ) -> Option<serde_json::Value> {
        use crate::managers::mcp_protocol::ToolContent;
        for content in &result.content {
            if let ToolContent::Text { text } = content {
                return serde_json::from_str(text).ok();
            }
        }
        None
    }

    async fn emit_event(&self, trace_id: ClotoId, data: ClotoEventData) {
        let envelope = crate::EnvelopedEvent {
            event: Arc::new(ClotoEvent::with_trace(trace_id, data)),
            issuer: None,
            correlation_id: Some(trace_id),
            depth: 0,
        };
        if let Err(e) = self.sender.send(envelope).await {
            warn!("⚠️ Failed to emit observability event: {}", e);
        }
    }
}
