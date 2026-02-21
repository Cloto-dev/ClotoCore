use async_trait::async_trait;
use chrono::Utc;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};

use crate::managers::{AgentManager, PluginRegistry};
use exiv_shared::{
    AgentMetadata, ExivEvent, ExivEventData, ExivId, ExivMessage, Plugin, PluginCast,
    PluginManifest, ThinkResult,
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
}

impl SystemHandler {
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
        }
    }

    pub async fn handle_message(&self, msg: ExivMessage) -> anyhow::Result<()> {
        let target_agent_id = msg
            .metadata
            .get("target_agent_id")
            .cloned()
            .unwrap_or_else(|| self.default_agent_id.clone());

        // 1. エージェント情報の取得
        let (agent, _engine_id) = self
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

        // 1b. エージェントに割り当てられたプラグインIDを取得
        let agent_plugin_ids: Vec<String> = self
            .agent_manager
            .get_agent_plugins(&target_agent_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|r| r.plugin_id)
            .collect();

        // 2. メモリからのコンテキスト取得
        let memory_plugin = if let Some(preferred_id) = agent.metadata.get("preferred_memory") {
            self.registry.get_engine(preferred_id).await
        } else {
            self.registry.find_memory().await
        };

        let context = if let Some(ref plugin) = memory_plugin {
            if let Some(mem) = plugin.as_memory() {
                // 🔐 Check MemoryRead permission before recall
                let manifest = plugin.manifest();
                let perms_lock = self.registry.effective_permissions.read().await;
                let plugin_exiv_id = exiv_shared::ExivId::from_name(&manifest.id);
                let has_memory_read = perms_lock
                    .get(&plugin_exiv_id)
                    .map(|p| p.contains(&exiv_shared::Permission::MemoryRead))
                    .unwrap_or(false);
                drop(perms_lock);
                if !has_memory_read {
                    tracing::warn!(
                        plugin_id = %manifest.id,
                        "⚠️  Memory plugin lacks MemoryRead permission — context recall skipped"
                    );
                    vec![]
                } else {
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
                } // end has_memory_read else branch
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        // 3. 【核心】思考要求イベントを発行
        info!(
            target_agent_id = %target_agent_id,
            agent_name = %agent.name,
            engine_id = %_engine_id,
            "📢 Dispatching Thought/Consensus Request"
        );

        let trace_id = exiv_shared::ExivId::new_trace_id();

        if msg.content.to_lowercase().starts_with("consensus:") {
            // 合意形成モード
            let thought_event_data = exiv_shared::ExivEventData::ConsensusRequested {
                task: msg.content.clone(),
                engine_ids: self.consensus_engines.clone(),
            };

            let envelope = crate::EnvelopedEvent {
                event: Arc::new(exiv_shared::ExivEvent::with_trace(
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
                let inner_thought = exiv_shared::ExivEventData::ThoughtRequested {
                    agent: agent.clone(),
                    engine_id: engine.clone(),
                    message: msg.clone(),
                    context: context.clone(),
                };
                let env = crate::EnvelopedEvent {
                    event: Arc::new(exiv_shared::ExivEvent::with_trace(trace_id, inner_thought)),
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
            let engine_id = _engine_id;
            match self
                .run_agentic_loop(
                    &agent,
                    &engine_id,
                    &msg,
                    context,
                    &agent_plugin_ids,
                    trace_id,
                )
                .await
            {
                Ok(content) => {
                    // エージェント返答もメモリに保存 (user messageと対で保存)
                    if let Some(plugin) = &memory_plugin {
                        let plugin_clone = plugin.clone();
                        let agent_resp_msg = ExivMessage {
                            id: format!("{}-resp", msg.id),
                            source: exiv_shared::MessageSource::Agent {
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
                    }

                    let thought_response = ExivEventData::ThoughtResponse {
                        agent_id: agent.id.clone(),
                        engine_id: engine_id.clone(),
                        content,
                        source_message_id: msg.id.clone(),
                    };
                    let envelope = crate::EnvelopedEvent {
                        event: Arc::new(ExivEvent::with_trace(trace_id, thought_response)),
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
                    let error_response = ExivEventData::ThoughtResponse {
                        agent_id: agent.id.clone(),
                        engine_id: engine_id.clone(),
                        content: format!("[Error] Processing failed: {}", e),
                        source_message_id: msg.id.clone(),
                    };
                    let envelope = crate::EnvelopedEvent {
                        event: Arc::new(ExivEvent::with_trace(trace_id, error_response)),
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
                    let pid = exiv_shared::ExivId::from_name(&manifest.id);
                    perms_lock
                        .get(&pid)
                        .map(|p| p.contains(&exiv_shared::Permission::MemoryWrite))
                        .unwrap_or(false)
                };
                if !has_memory_write {
                    tracing::warn!(
                        plugin_id = %manifest.id,
                        "⚠️  Memory plugin lacks MemoryWrite permission — store skipped"
                    );
                } else {
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
                } // end has_memory_write branch
            }
        }

        Ok(())
    }

    // ── Agentic Loop ──

    async fn run_agentic_loop(
        &self,
        agent: &AgentMetadata,
        engine_id: &str,
        message: &ExivMessage,
        context: Vec<ExivMessage>,
        agent_plugin_ids: &[String],
        trace_id: ExivId,
    ) -> anyhow::Result<String> {
        let engine_plugin = self
            .registry
            .get_engine(engine_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("Engine '{}' not found", engine_id))?;
        let engine = engine_plugin
            .as_reasoning()
            .ok_or_else(|| anyhow::anyhow!("Plugin '{}' is not a ReasoningEngine", engine_id))?;

        // Fallback: engine does not support tools → plain think()
        if !engine.supports_tools() {
            return engine.think(agent, message, context).await;
        }

        // エージェントに割り当てられたプラグインのみからツールを収集
        let tools = if agent_plugin_ids.is_empty() {
            self.registry.collect_tool_schemas().await
        } else {
            self.registry
                .collect_tool_schemas_for(agent_plugin_ids)
                .await
        };
        if tools.is_empty() {
            return engine.think(agent, message, context).await;
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
                return engine.think(agent, message, context.clone()).await;
            }

            let result = engine
                .think_with_tools(agent, message, context.clone(), &tools, &tool_history)
                .await?;

            match result {
                ThinkResult::Final(content) => {
                    // Emit loop completion event
                    self.emit_event(
                        trace_id,
                        ExivEventData::AgenticLoopCompleted {
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

                        let tool_result = tokio::time::timeout(
                            Duration::from_secs(self.tool_execution_timeout_secs),
                            async {
                                if agent_plugin_ids.is_empty() {
                                    self.registry
                                        .execute_tool(&call.name, call.arguments.clone())
                                        .await
                                } else {
                                    self.registry
                                        .execute_tool_for(
                                            agent_plugin_ids,
                                            &call.name,
                                            call.arguments.clone(),
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
                            ExivEventData::ToolInvoked {
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

    async fn emit_event(&self, trace_id: ExivId, data: ExivEventData) {
        let envelope = crate::EnvelopedEvent {
            event: Arc::new(ExivEvent::with_trace(trace_id, data)),
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
            id: "core.system".to_string(),
            name: "Kernel System Handler".to_string(),
            description: "Internal core logic handler".to_string(),
            version: "1.0.0".to_string(),
            category: exiv_shared::PluginCategory::System,
            service_type: exiv_shared::ServiceType::Reasoning,
            tags: vec![],
            is_active: true,
            is_configured: true,
            required_config_keys: vec![],
            action_icon: None,
            action_target: None,
            icon_data: None,
            magic_seal: 0x56455253,
            sdk_version: "internal".to_string(),
            required_permissions: vec![],
            provided_capabilities: vec![],
            provided_tools: vec![],
        }
    }

    async fn on_event(
        &self,
        event: &ExivEvent,
    ) -> anyhow::Result<Option<exiv_shared::ExivEventData>> {
        if let exiv_shared::ExivEventData::MessageReceived(msg) = &event.data {
            // Only trigger thinking for messages from users to prevent agent-agent loops
            if matches!(msg.source, exiv_shared::MessageSource::User { .. }) {
                let msg = msg.clone();
                self.handle_message(msg).await?;
            }
        }
        Ok(None)
    }
}
