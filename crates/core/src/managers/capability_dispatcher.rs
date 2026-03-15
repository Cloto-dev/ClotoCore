//! Capability-based tool routing for the ClotoCore kernel.
//!
//! Maps abstract capabilities (Memory, Reasoning, Vision) to concrete
//! MCP server/tool pairs, eliminating hard-coded server IDs from the kernel
//! (P1 Core Minimalism compliance).

use std::collections::HashMap;
use tokio::sync::RwLock;
use tracing::debug;

/// Abstract capability types that the kernel can dispatch to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CapabilityType {
    /// Memory storage and recall (store, recall, list_memories, etc.)
    Memory,
    /// LLM reasoning engines (think, think_with_tools)
    Reasoning,
    /// Vision/image analysis (analyze_image)
    Vision,
    /// Speech-to-text (transcribe)
    Stt,
}

/// A mapping from capability to a specific server/tool pair.
#[derive(Debug, Clone)]
struct CapabilityMapping {
    server_id: String,
    tool_name: String,
    #[allow(dead_code)]
    priority: u8,
}

/// Routes kernel capability requests to the appropriate MCP server.
///
/// Built dynamically as MCP servers connect and disconnect, this eliminates
/// the need for hard-coded server IDs throughout the kernel codebase.
pub struct CapabilityDispatcher {
    mappings: RwLock<HashMap<CapabilityType, Vec<CapabilityMapping>>>,
}

impl Default for CapabilityDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl CapabilityDispatcher {
    #[must_use]
    pub fn new() -> Self {
        Self {
            mappings: RwLock::new(HashMap::new()),
        }
    }

    /// Register capabilities from a newly connected server's tool list.
    pub async fn build_from_tools(&self, server_id: &str, tools: &[super::mcp_protocol::McpTool]) {
        let mut mappings = self.mappings.write().await;
        for tool in tools {
            if let Some(cap) = classify_tool(server_id, &tool.name) {
                let entries = mappings.entry(cap).or_default();
                if !entries
                    .iter()
                    .any(|m| m.server_id == server_id && m.tool_name == tool.name)
                {
                    entries.push(CapabilityMapping {
                        server_id: server_id.to_string(),
                        tool_name: tool.name.clone(),
                        priority: 0,
                    });
                }
            }
        }
        debug!(server = %server_id, "Capability mappings updated");
    }

    /// Remove all mappings for a disconnected server.
    pub async fn remove_server(&self, server_id: &str) {
        let mut mappings = self.mappings.write().await;
        for entries in mappings.values_mut() {
            entries.retain(|m| m.server_id != server_id);
        }
        debug!(server = %server_id, "Capability mappings removed");
    }

    /// Resolve a capability to the server that provides the given tool.
    pub async fn resolve(
        &self,
        capability: CapabilityType,
        tool_name: &str,
    ) -> Option<(String, String)> {
        let mappings = self.mappings.read().await;
        if let Some(entries) = mappings.get(&capability) {
            for entry in entries {
                if entry.tool_name == tool_name {
                    return Some((entry.server_id.clone(), entry.tool_name.clone()));
                }
            }
        }
        None
    }

    /// Resolve any server providing this capability (returns highest-priority match).
    pub async fn resolve_server(&self, capability: CapabilityType) -> Option<String> {
        let mappings = self.mappings.read().await;
        mappings
            .get(&capability)
            .and_then(|entries| entries.first())
            .map(|m| m.server_id.clone())
    }

    /// Resolve all providers for a capability.
    pub async fn resolve_all(&self, capability: CapabilityType) -> Vec<(String, String)> {
        let mappings = self.mappings.read().await;
        mappings
            .get(&capability)
            .map(|entries| {
                entries
                    .iter()
                    .map(|m| (m.server_id.clone(), m.tool_name.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Classify a server/tool combination into a CapabilityType.
fn classify_tool(server_id: &str, tool_name: &str) -> Option<CapabilityType> {
    // Server prefix classification (primary)
    if server_id.starts_with("memory.") {
        return Some(CapabilityType::Memory);
    }
    if server_id.starts_with("mind.") {
        return Some(CapabilityType::Reasoning);
    }
    if server_id.starts_with("vision.") {
        return Some(CapabilityType::Vision);
    }
    if server_id.starts_with("stt.") {
        return Some(CapabilityType::Stt);
    }

    // Tool name fallback for non-standard server prefixes
    match tool_name {
        "store" | "recall" | "list_memories" | "delete_memory" | "list_episodes"
        | "delete_episode" | "archive_episode" | "delete_agent_data" | "update_profile" => {
            Some(CapabilityType::Memory)
        }
        "think" | "think_with_tools" => Some(CapabilityType::Reasoning),
        "analyze_image" | "capture_screenshot" => Some(CapabilityType::Vision),
        "transcribe" => Some(CapabilityType::Stt),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::mcp_protocol::McpTool;

    fn make_tool(name: &str) -> McpTool {
        McpTool {
            name: name.to_string(),
            description: None,
            input_schema: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn test_build_and_resolve() {
        let dispatcher = CapabilityDispatcher::new();
        let tools = vec![
            make_tool("store"),
            make_tool("recall"),
            make_tool("list_memories"),
        ];
        dispatcher.build_from_tools("memory.cpersona", &tools).await;

        let result = dispatcher.resolve(CapabilityType::Memory, "recall").await;
        assert!(result.is_some());
        let (server, tool) = result.unwrap();
        assert_eq!(server, "memory.cpersona");
        assert_eq!(tool, "recall");
    }

    #[tokio::test]
    async fn test_resolve_server() {
        let dispatcher = CapabilityDispatcher::new();
        let tools = vec![make_tool("analyze_image")];
        dispatcher.build_from_tools("vision.capture", &tools).await;

        let server = dispatcher.resolve_server(CapabilityType::Vision).await;
        assert_eq!(server.as_deref(), Some("vision.capture"));
    }

    #[tokio::test]
    async fn test_remove_server() {
        let dispatcher = CapabilityDispatcher::new();
        let tools = vec![make_tool("store"), make_tool("recall")];
        dispatcher.build_from_tools("memory.cpersona", &tools).await;

        dispatcher.remove_server("memory.cpersona").await;

        assert!(dispatcher
            .resolve_server(CapabilityType::Memory)
            .await
            .is_none());
    }

    #[tokio::test]
    async fn test_classify_by_tool_name() {
        let dispatcher = CapabilityDispatcher::new();
        let tools = vec![make_tool("store"), make_tool("recall")];
        dispatcher.build_from_tools("custom.storage", &tools).await;

        let result = dispatcher.resolve(CapabilityType::Memory, "recall").await;
        assert!(result.is_some());
    }

    #[tokio::test]
    async fn test_no_duplicates() {
        let dispatcher = CapabilityDispatcher::new();
        let tools = vec![make_tool("store")];
        dispatcher.build_from_tools("memory.cpersona", &tools).await;
        dispatcher.build_from_tools("memory.cpersona", &tools).await;

        let all = dispatcher.resolve_all(CapabilityType::Memory).await;
        assert_eq!(all.len(), 1);
    }
}
