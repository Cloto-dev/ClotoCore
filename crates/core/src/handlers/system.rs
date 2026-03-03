use async_trait::async_trait;
use chrono::Utc;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};

use crate::managers::{AgentManager, McpClientManager, PluginRegistry};
use dashmap::DashMap;
use sqlx::SqlitePool;
use std::collections::HashSet;
use tokio::sync::oneshot;
use cloto_shared::{
    AgentMetadata, ClotoEvent, ClotoEventData, ClotoId, ClotoMessage, Plugin, PluginCast,
    PluginManifest, ThinkResult, ToolCall,
};

// ── Engine Routing Rules ──

#[derive(serde::Deserialize)]
struct RoutingRule {
    #[serde(rename = "match")]
    condition: String,
    engine: String,
}

impl RoutingRule {
    fn matches(&self, message: &str) -> bool {
        if self.condition == "default" {
            return true;
        }
        if let Some(kw) = self.condition.strip_prefix("contains:") {
            return message.to_lowercase().contains(&kw.to_lowercase());
        }
        if let Some(len) = self.condition.strip_prefix("length:>") {
            if let Ok(n) = len.parse::<usize>() {
                return message.len() > n;
            }
        }
        if self.condition == "tools_likely" {
            let tool_keywords = [
                "調べ",
                "検索",
                "実行",
                "ファイル",
                "search",
                "run",
                "execute",
                "find",
                "research",
            ];
            return tool_keywords
                .iter()
                .any(|k| message.to_lowercase().contains(k));
        }
        false
    }
}

fn evaluate_engine_routing(
    message: &str,
    metadata: &std::collections::HashMap<String, String>,
    connected_servers: &[String],
) -> Option<String> {
    let rules_json = metadata.get("engine_routing")?;
    let rules: Vec<RoutingRule> = serde_json::from_str(rules_json).ok()?;

    for rule in &rules {
        if !connected_servers.contains(&rule.engine) {
            continue;
        }
        if rule.matches(message) {
            return Some(rule.engine.clone());
        }
    }
    None
}

/// User's decision on a command approval request.
#[derive(Debug, Clone, PartialEq)]
pub enum CommandApprovalDecision {
    Approve,
    Trust,
    Deny,
}

/// In-memory pending command approval requests (approval_id → oneshot sender).
pub type PendingApprovals = Arc<DashMap<String, oneshot::Sender<CommandApprovalDecision>>>;

/// Session-scoped trusted command names (agent_id → set of command names).
/// Cleared on kernel restart.
pub type SessionTrustedCommands = Arc<DashMap<String, HashSet<String>>>;

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
        }
    }

    #[allow(clippy::too_many_lines)]
    pub async fn handle_message(&self, msg: ClotoMessage) -> anyhow::Result<()> {
        let target_agent_id = msg
            .metadata
            .get("target_agent_id")
            .cloned()
            .unwrap_or_else(|| self.default_agent_id.clone());

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
        self.agent_manager
            .touch_last_seen(&target_agent_id)
            .await
            .ok();

        // Persist user message to chat history (backend-side persistence)
        let now_ms = chrono::Utc::now().timestamp_millis();
        let user_chat_msg = crate::db::ChatMessageRow {
            id: msg.id.clone(),
            agent_id: target_agent_id.clone(),
            user_id: "default".to_string(),
            source: "user".to_string(),
            content: serde_json::to_string(
                &serde_json::json!([{"type": "text", "text": &msg.content}])
            ).unwrap_or_default(),
            metadata: None,
            created_at: now_ms,
        };
        if let Err(e) = crate::db::save_chat_message(&self.pool, &user_chat_msg).await {
            warn!("Failed to persist user message: {}", e);
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
                mcp.find_memory_server().await.and_then(|server_id| {
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
                let perms_lock = self.registry.effective_permissions.read().await;
                let plugin_cloto_id = cloto_shared::ClotoId::from_name(&manifest.id);
                let has_memory_read = perms_lock
                    .get(&plugin_cloto_id)
                    .is_some_and(|p| p.contains(&cloto_shared::Permission::MemoryRead));
                drop(perms_lock);
                if has_memory_read {
                    // 🛑 停滞対策: メモリの呼び出しにタイムアウトを設定
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(5),
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
                std::time::Duration::from_secs(5),
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
            let engine_id = if let Some(ov) = msg.metadata.get("engine_override") {
                ov.clone()
            } else if let Some(ref mcp) = self.registry.mcp_manager {
                let connected = mcp.list_connected_mind_servers().await;
                evaluate_engine_routing(&msg.content, &agent.metadata, &connected)
                    .unwrap_or(default_engine_id)
            } else {
                evaluate_engine_routing(&msg.content, &agent.metadata, &[])
                    .unwrap_or(default_engine_id)
            };
            match self
                .run_agentic_loop(
                    &agent,
                    &engine_id,
                    &msg,
                    context,
                    &granted_server_ids,
                    trace_id,
                )
                .await
            {
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
                        tokio::spawn(async move {
                            if let Some(mem) = plugin_clone.as_memory() {
                                let _ = tokio::time::timeout(
                                    std::time::Duration::from_secs(5),
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
                        tokio::spawn(async move {
                            let store_args = serde_json::json!({
                                "agent_id": agent_id_clone,
                                "message": resp_msg_json,
                            });
                            let _ = tokio::time::timeout(
                                std::time::Duration::from_secs(5),
                                mcp_clone.call_server_tool(&server_id_clone, "store", store_args),
                            )
                            .await;
                        });
                    }

                    // Persist agent response to chat history (backend-side)
                    let resp_id = format!("{}-resp", msg.id);
                    let agent_chat_msg = crate::db::ChatMessageRow {
                        id: resp_id,
                        agent_id: agent.id.clone(),
                        user_id: "default".to_string(),
                        source: "agent".to_string(),
                        content: serde_json::to_string(
                            &serde_json::json!([{"type": "text", "text": &content}])
                        ).unwrap_or_default(),
                        metadata: None,
                        created_at: chrono::Utc::now().timestamp_millis(),
                    };
                    if let Err(e) = crate::db::save_chat_message(&self.pool, &agent_chat_msg).await {
                        warn!("Failed to persist agent response: {}", e);
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
                    let err_chat_msg = crate::db::ChatMessageRow {
                        id: err_resp_id,
                        agent_id: agent.id.clone(),
                        user_id: "default".to_string(),
                        source: "agent".to_string(),
                        content: serde_json::to_string(
                            &serde_json::json!([{"type": "text", "text": &error_content}])
                        ).unwrap_or_default(),
                        metadata: None,
                        created_at: chrono::Utc::now().timestamp_millis(),
                    };
                    let _ = crate::db::save_chat_message(&self.pool, &err_chat_msg).await;

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
                    let perms_lock = self.registry.effective_permissions.read().await;
                    let pid = cloto_shared::ClotoId::from_name(&manifest.id);
                    perms_lock
                        .get(&pid)
                        .is_some_and(|p| p.contains(&cloto_shared::Permission::MemoryWrite))
                };
                if has_memory_write {
                    let agent_id = agent.id.clone();
                    let plugin_clone = plugin.clone();
                    let metrics = self.metrics.clone();
                    // 🛑 停滞対策: 保存処理はバックグラウンドで行い、メインループをブロックしない
                    tokio::spawn(async move {
                        if let Some(mem) = plugin_clone.as_memory() {
                            match tokio::time::timeout(
                                std::time::Duration::from_secs(5),
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

            tokio::spawn(async move {
                let store_args = serde_json::json!({
                    "agent_id": agent_id,
                    "message": msg_json,
                });
                match tokio::time::timeout(
                    std::time::Duration::from_secs(5),
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
                        error!(agent_id = %agent_id, "❌ MCP memory store timed out (5s)");
                    }
                }
            });

            // Episode auto-archival check (background, non-blocking)
            tokio::spawn(async move {
                Self::maybe_archive_episode(&ep_mcp, &ep_server_id, &ep_agent_id).await;
            });
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
        const MAX_TOOL_HISTORY: usize = 100;

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
                    }
                    tool_history.push(assistant_msg);

                    // Execute each tool call
                    for call in &calls {
                        total_tool_calls += 1;

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

                        // 🔐 Anti-spoofing: force agent_id in tool arguments
                        // Prevents LLM from specifying a different agent's ID
                        // to access their memory or profile
                        let mut safe_args = call.arguments.clone();
                        if let Some(obj) = safe_args.as_object_mut() {
                            if obj.contains_key("agent_id") {
                                obj.insert(
                                    "agent_id".to_string(),
                                    serde_json::Value::String(agent.id.clone()),
                                );
                            }
                        }

                        // ── Command Approval Gate ──
                        if call.name == "execute_command" {
                            if let Some(command_str) = safe_args.get("command").and_then(|v| v.as_str()) {
                                let yolo = self.registry.mcp_manager.as_ref()
                                    .map_or(false, |m| m.yolo_mode.load(std::sync::atomic::Ordering::Relaxed));

                                if !yolo {
                                    // Check DB exact match
                                    let db_trusted = crate::db::is_command_trusted(
                                        &self.pool, &agent.id, command_str
                                    ).await.unwrap_or(false);

                                    // Check session command-name trust
                                    let cmd_name = command_str.split_whitespace().next().unwrap_or(command_str);
                                    let session_trusted = self.session_trusted_commands
                                        .get(&agent.id)
                                        .map_or(false, |set| set.contains(cmd_name));

                                    if !db_trusted && !session_trusted {
                                        let approval_id = uuid::Uuid::new_v4().to_string();
                                        info!(agent_id = %agent.id, command = %command_str, "🔒 Command requires approval");

                                        let (atx, arx) = oneshot::channel();
                                        self.pending_approvals.insert(approval_id.clone(), atx);

                                        self.emit_event(trace_id, ClotoEventData::CommandApprovalRequested {
                                            approval_id: approval_id.clone(),
                                            agent_id: agent.id.clone(),
                                            command: command_str.to_string(),
                                            command_name: cmd_name.to_string(),
                                        }).await;

                                        let decision = tokio::time::timeout(Duration::from_secs(60), arx).await;
                                        self.pending_approvals.remove(&approval_id);

                                        match decision {
                                            Ok(Ok(CommandApprovalDecision::Approve)) => {
                                                // Store exact match in DB for future sessions
                                                let _ = crate::db::add_trusted_command(
                                                    &self.pool, &agent.id, command_str,
                                                ).await;
                                                info!(approval_id = %approval_id, "✅ Command approved (exact)");
                                                self.emit_event(trace_id, ClotoEventData::CommandApprovalResult {
                                                    approval_id, decision: "approved".to_string(),
                                                }).await;
                                            }
                                            Ok(Ok(CommandApprovalDecision::Trust)) => {
                                                // Store command name in session memory only
                                                self.session_trusted_commands
                                                    .entry(agent.id.clone())
                                                    .or_default()
                                                    .insert(cmd_name.to_string());
                                                info!(approval_id = %approval_id, cmd = %cmd_name, "✅ Command name trusted (session)");
                                                self.emit_event(trace_id, ClotoEventData::CommandApprovalResult {
                                                    approval_id, decision: "trusted".to_string(),
                                                }).await;
                                            }
                                            _ => {
                                                let reason = if matches!(decision, Err(_)) { "timeout (60s)" } else { "denied by user" };
                                                warn!(approval_id = %approval_id, reason = reason, "🚫 Command denied");
                                                self.emit_event(trace_id, ClotoEventData::CommandApprovalResult {
                                                    approval_id, decision: reason.to_string(),
                                                }).await;
                                                tool_history.push(serde_json::json!({
                                                    "role": "tool",
                                                    "tool_call_id": call.id,
                                                    "content": format!("Error: command '{}' was {}", command_str, reason)
                                                }));
                                                continue;
                                            }
                                        }
                                    }
                                }
                            }
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
            let temp_path = format!(
                "data/tmp_vision_{}.{ext}",
                uuid::Uuid::new_v4()
            );
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
                .call_server_tool("vision.capture", "analyze_image", args)
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
                        return Err(anyhow::anyhow!("{}", Self::format_engine_error(error, code)));
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
                    return Err(anyhow::anyhow!("{}", Self::format_engine_error(error, code)));
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

    /// Parse MCP recall() response into Vec<ClotoMessage>.
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
                        return messages
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
                    }
                }
            }
        }
        vec![]
    }

    /// Auto-archive episode when enough unarchived memories accumulate.
    async fn maybe_archive_episode(mcp: &Arc<McpClientManager>, server_id: &str, agent_id: &str) {
        const THRESHOLD: usize = 10;

        // 1. Fetch recent memories
        let Ok(Ok(mem_result)) = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            mcp.call_server_tool(
                server_id,
                "list_memories",
                serde_json::json!({"agent_id": agent_id, "limit": THRESHOLD + 5}),
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
            Some(m) if m.len() >= THRESHOLD => m,
            _ => return,
        };

        // 2. Get last episode timestamp
        let Ok(Ok(ep_result)) = tokio::time::timeout(
            std::time::Duration::from_secs(5),
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

        if unarchived.len() < THRESHOLD {
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

impl PluginCast for SystemHandler {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[async_trait]
impl Plugin for SystemHandler {
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "kernel.system".to_string(),
            name: "Kernel System Handler".to_string(),
            description: "Internal core logic handler".to_string(),
            version: "1.0.0".to_string(),
            category: cloto_shared::PluginCategory::System,
            service_type: cloto_shared::ServiceType::Reasoning,
            tags: vec![],
            is_active: true,
            is_configured: true,
            required_config_keys: vec![],
            action_icon: None,
            action_target: None,
            icon_data: None,
            magic_seal: 0x5645_5253,
            sdk_version: "internal".to_string(),
            required_permissions: vec![],
            provided_capabilities: vec![],
            provided_tools: vec![],
        }
    }

    async fn on_event(
        &self,
        event: &ClotoEvent,
    ) -> anyhow::Result<Option<cloto_shared::ClotoEventData>> {
        if let cloto_shared::ClotoEventData::MessageReceived(msg) = &event.data {
            // Only trigger thinking for messages from users to prevent agent-agent loops
            if matches!(msg.source, cloto_shared::MessageSource::User { .. }) {
                let msg = msg.clone();
                self.handle_message(msg).await?;
            }
        }
        Ok(None)
    }
}
