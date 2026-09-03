//! Capability-based tool routing for the ClotoCore kernel.
//!
//! Maps abstract capabilities (Memory, Reasoning, Vision) to concrete
//! MCP server/tool pairs, eliminating hard-coded server IDs from the kernel
//! (P1 Core Minimalism compliance).

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
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
    /// Speech output / text-to-speech (speak)
    Speech,
}

impl fmt::Display for CapabilityType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            CapabilityType::Memory => "Memory",
            CapabilityType::Reasoning => "Reasoning",
            CapabilityType::Vision => "Vision",
            CapabilityType::Stt => "Stt",
            CapabilityType::Speech => "Speech",
        };
        f.write_str(s)
    }
}

impl FromStr for CapabilityType {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "Memory" => Ok(CapabilityType::Memory),
            "Reasoning" => Ok(CapabilityType::Reasoning),
            "Vision" => Ok(CapabilityType::Vision),
            "Stt" => Ok(CapabilityType::Stt),
            "Speech" => Ok(CapabilityType::Speech),
            _ => Err(()),
        }
    }
}

/// Well-known tool names the kernel dispatches by enum (no string literals in handlers).
///
/// The 11 named variants cover every tool the kernel currently calls or checks for via
/// `handlers/system.rs`. `Custom(String)` is an escape hatch for explicit-target calls
/// that don't fit a well-known capability (e.g. `get_profile` at single use sites).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ToolKind {
    // Memory (7)
    Store,
    Recall,
    ListMemories,
    ListEpisodes,
    ArchiveEpisode,
    UpdateProfile,
    DeleteAgentData,
    // Reasoning (2)
    Think,
    ThinkWithTools,
    // Vision (1)
    AnalyzeImage,
    // Stt (1)
    Transcribe,
    // Speech (1)
    Speak,
    /// Escape hatch — wire-level tool name without a well-known variant.
    /// `capability()` returns `None`, so this variant must be paired with an
    /// explicit `server_id` (use `call_kind_at` / `has_kind_at` family).
    Custom(String),
}

impl ToolKind {
    /// Wire-level MCP tool name as transmitted over MGP.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            ToolKind::Store => "store",
            ToolKind::Recall => "recall",
            ToolKind::ListMemories => "list_memories",
            ToolKind::ListEpisodes => "list_episodes",
            ToolKind::ArchiveEpisode => "archive_episode",
            ToolKind::UpdateProfile => "update_profile",
            ToolKind::DeleteAgentData => "delete_agent_data",
            ToolKind::Think => "think",
            ToolKind::ThinkWithTools => "think_with_tools",
            ToolKind::AnalyzeImage => "analyze_image",
            ToolKind::Transcribe => "transcribe",
            ToolKind::Speak => "speak",
            ToolKind::Custom(s) => s,
        }
    }

    /// Which capability provides this kind. `Custom` returns `None` — callers
    /// must route via explicit `server_id`.
    #[must_use]
    pub fn capability(&self) -> Option<CapabilityType> {
        match self {
            ToolKind::Store
            | ToolKind::Recall
            | ToolKind::ListMemories
            | ToolKind::ListEpisodes
            | ToolKind::ArchiveEpisode
            | ToolKind::UpdateProfile
            | ToolKind::DeleteAgentData => Some(CapabilityType::Memory),
            ToolKind::Think | ToolKind::ThinkWithTools => Some(CapabilityType::Reasoning),
            ToolKind::AnalyzeImage => Some(CapabilityType::Vision),
            ToolKind::Transcribe => Some(CapabilityType::Stt),
            ToolKind::Speak => Some(CapabilityType::Speech),
            ToolKind::Custom(_) => None,
        }
    }

    /// Reverse map: build a `ToolKind` from an MCP tool name. Unknown names
    /// fall through to `Custom(name)`.
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name {
            "store" => ToolKind::Store,
            "recall" => ToolKind::Recall,
            "list_memories" => ToolKind::ListMemories,
            "list_episodes" => ToolKind::ListEpisodes,
            "archive_episode" => ToolKind::ArchiveEpisode,
            "update_profile" => ToolKind::UpdateProfile,
            "delete_agent_data" => ToolKind::DeleteAgentData,
            "think" => ToolKind::Think,
            "think_with_tools" => ToolKind::ThinkWithTools,
            "analyze_image" => ToolKind::AnalyzeImage,
            "transcribe" => ToolKind::Transcribe,
            "speak" => ToolKind::Speak,
            _ => ToolKind::Custom(name.to_string()),
        }
    }
}

/// A mapping from capability to a specific server/tool pair.
#[derive(Debug, Clone)]
struct CapabilityMapping {
    server_id: String,
    tool_name: String,
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

    /// Register capabilities from a newly connected server's tool list using
    /// the legacy `classify_tool` heuristic (server prefix + tool name match).
    ///
    /// Use [`build_from_capabilities`] instead when the server declares its
    /// own `tools_for_capability` mapping in its MGP `initialize` response.
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
                    });
                }
            }
        }
        debug!(server = %server_id, "Capability mappings updated");
    }

    /// Register capabilities from a server-declared manifest
    /// (`MgpServerCapabilities.tools_for_capability`, Pattern-C, 0.6.8-alpha.2).
    ///
    /// Keys must parse as [`CapabilityType`] via `FromStr` (`"Memory"`,
    /// `"Reasoning"`, `"Vision"`, `"Stt"`, `"Speech"`). Unknown keys are
    /// logged and skipped — the kernel never invents capabilities from
    /// arbitrary server strings.
    ///
    /// This is the manifest-driven path that replaces the `classify_tool`
    /// heuristic for servers that opt in. Both paths can coexist in the
    /// same dispatcher (different servers).
    pub async fn build_from_capabilities(
        &self,
        server_id: &str,
        tools_for_capability: &HashMap<String, Vec<String>>,
    ) {
        let mut mappings = self.mappings.write().await;
        for (cap_name, tool_names) in tools_for_capability {
            let Ok(cap) = CapabilityType::from_str(cap_name) else {
                tracing::warn!(
                    server = %server_id,
                    capability = %cap_name,
                    "tools_for_capability declared unknown CapabilityType; ignoring",
                );
                continue;
            };
            let entries = mappings.entry(cap).or_default();
            for tool_name in tool_names {
                if !entries
                    .iter()
                    .any(|m| m.server_id == server_id && &m.tool_name == tool_name)
                {
                    entries.push(CapabilityMapping {
                        server_id: server_id.to_string(),
                        tool_name: tool_name.clone(),
                    });
                }
            }
        }
        debug!(server = %server_id, "Capability mappings updated from manifest");
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

    /// Resolve any server providing this capability (returns the first registered match).
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

    /// Resolve a `ToolKind` to the server that provides it.
    ///
    /// For well-known variants this is equivalent to `resolve(kind.capability()?, kind.name())`.
    /// `Custom(_)` returns `None` because there is no capability to filter on — use
    /// explicit-target shims (`call_kind_at`) for those.
    pub async fn resolve_kind(&self, kind: &ToolKind) -> Option<(String, String)> {
        let cap = kind.capability()?;
        self.resolve(cap, kind.name()).await
    }

    /// True if any registered server provides this `ToolKind` under its capability.
    pub async fn has_kind(&self, kind: &ToolKind) -> bool {
        self.resolve_kind(kind).await.is_some()
    }
}

/// Classify a server/tool combination into a CapabilityType.
///
/// Classification is tool-surface only: the tool name is the sole signal
/// (docs/CATEGORY_PREFIX_RETIREMENT_DESIGN.md §5). Server ids carry no
/// category semantics — the historical `memory.` / `mind.` / `vision.` /
/// `stt.` / `output.` prefix arms were retired along with the prefixed ids
/// themselves; dotted ids remain legal but meaningless.
fn classify_tool(_server_id: &str, tool_name: &str) -> Option<CapabilityType> {
    match tool_name {
        "store"
        | "recall"
        | "list_memories"
        | "delete_memory"
        | "list_episodes"
        | "delete_episode"
        | "archive_episode"
        | "delete_agent_data"
        | "update_profile"
        | "update_memory"
        | "lock_memory"
        | "unlock_memory"
        | "set_recall_precision"
        | "get_recall_precision" => Some(CapabilityType::Memory),
        "think" | "think_with_tools" => Some(CapabilityType::Reasoning),
        // `capture_screen` is the capture server's real screenshot tool;
        // `capture_screenshot` is kept for compatibility with third parties.
        "analyze_image" | "capture_screen" | "capture_screenshot" => Some(CapabilityType::Vision),
        "transcribe" => Some(CapabilityType::Stt),
        "speak" => Some(CapabilityType::Speech),
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
            annotations: None,
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
    async fn test_bare_ids_classify_by_tool_surface() {
        // Category prefixes are retired: bare catalog ids must
        // classify purely from their tool surface.
        let dispatcher = CapabilityDispatcher::new();
        dispatcher
            .build_from_tools(
                "capture",
                &[make_tool("capture_screen"), make_tool("analyze_image")],
            )
            .await;
        dispatcher
            .build_from_tools("stt", &[make_tool("transcribe")])
            .await;
        dispatcher
            .build_from_tools("avatar", &[make_tool("speak")])
            .await;

        assert_eq!(
            dispatcher
                .resolve(CapabilityType::Vision, "analyze_image")
                .await,
            Some(("capture".to_string(), "analyze_image".to_string()))
        );
        assert_eq!(
            dispatcher
                .resolve(CapabilityType::Vision, "capture_screen")
                .await,
            Some(("capture".to_string(), "capture_screen".to_string()))
        );
        assert_eq!(
            dispatcher.resolve(CapabilityType::Stt, "transcribe").await,
            Some(("stt".to_string(), "transcribe".to_string()))
        );
        assert_eq!(
            dispatcher.resolve(CapabilityType::Speech, "speak").await,
            Some(("avatar".to_string(), "speak".to_string()))
        );
    }

    #[tokio::test]
    async fn test_prefix_carries_no_category_semantics() {
        // A dotted id whose tools match no capability arm classifies as
        // nothing — the old `vision.` prefix arm must not resurrect it.
        let dispatcher = CapabilityDispatcher::new();
        dispatcher
            .build_from_tools(
                "vision.gaze_webcam",
                &[make_tool("start_tracking"), make_tool("stop_tracking")],
            )
            .await;

        assert!(dispatcher
            .resolve_server(CapabilityType::Vision)
            .await
            .is_none());
    }

    #[tokio::test]
    async fn test_set_recall_precision_is_memory_capability() {
        // A memory server with no `memory.` prefix (cpersona's id post bug-388)
        // must still classify the optional `set_recall_precision` op as Memory via
        // the tool-name whitelist, so feature detection (has_capability_tool) works.
        let dispatcher = CapabilityDispatcher::new();
        let tools = vec![make_tool("recall"), make_tool("set_recall_precision")];
        dispatcher.build_from_tools("cpersona", &tools).await;

        let result = dispatcher
            .resolve(CapabilityType::Memory, "set_recall_precision")
            .await;
        assert!(
            result.is_some(),
            "set_recall_precision should resolve as Memory"
        );
        let (server, tool) = result.unwrap();
        assert_eq!(server, "cpersona");
        assert_eq!(tool, "set_recall_precision");
    }

    #[tokio::test]
    async fn test_get_recall_precision_is_memory_capability() {
        // The read-back companion classifies as Memory via the same whitelist, so the
        // dashboard can feature-detect it and load an agent's current precision.
        let dispatcher = CapabilityDispatcher::new();
        let tools = vec![make_tool("recall"), make_tool("get_recall_precision")];
        dispatcher.build_from_tools("cpersona", &tools).await;

        let result = dispatcher
            .resolve(CapabilityType::Memory, "get_recall_precision")
            .await;
        assert!(
            result.is_some(),
            "get_recall_precision should resolve as Memory"
        );
        let (server, tool) = result.unwrap();
        assert_eq!(server, "cpersona");
        assert_eq!(tool, "get_recall_precision");
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

    // ============================================================
    // ToolKind enum tests (Pattern-C, 0.6.8-alpha.2)
    // ============================================================

    fn well_known_kinds() -> Vec<ToolKind> {
        vec![
            ToolKind::Store,
            ToolKind::Recall,
            ToolKind::ListMemories,
            ToolKind::ListEpisodes,
            ToolKind::ArchiveEpisode,
            ToolKind::UpdateProfile,
            ToolKind::Think,
            ToolKind::ThinkWithTools,
            ToolKind::AnalyzeImage,
            ToolKind::Transcribe,
            ToolKind::Speak,
        ]
    }

    #[test]
    fn test_kind_round_trip() {
        for kind in well_known_kinds() {
            let name = kind.name();
            let recovered = ToolKind::from_name(name);
            assert_eq!(recovered, kind, "round-trip failed for {kind:?}");
        }
    }

    #[tokio::test]
    async fn test_resolve_kind_memory() {
        let dispatcher = CapabilityDispatcher::new();
        let tools = vec![make_tool("store"), make_tool("recall")];
        dispatcher.build_from_tools("memory.cpersona", &tools).await;

        let result = dispatcher.resolve_kind(&ToolKind::Recall).await;
        assert!(result.is_some());
        let (server, tool) = result.unwrap();
        assert_eq!(server, "memory.cpersona");
        assert_eq!(tool, "recall");
        assert!(dispatcher.has_kind(&ToolKind::Recall).await);
        assert!(dispatcher.has_kind(&ToolKind::Store).await);
        assert!(!dispatcher.has_kind(&ToolKind::Think).await);
    }

    #[tokio::test]
    async fn test_resolve_kind_speech() {
        let dispatcher = CapabilityDispatcher::new();
        let tools = vec![make_tool("speak")];
        dispatcher.build_from_tools("output.coqui", &tools).await;

        let result = dispatcher.resolve_kind(&ToolKind::Speak).await;
        assert_eq!(
            result,
            Some(("output.coqui".to_string(), "speak".to_string()))
        );
    }

    #[tokio::test]
    async fn test_from_name_custom_falls_through() {
        let kind = ToolKind::from_name("get_profile");
        assert_eq!(kind, ToolKind::Custom("get_profile".to_string()));
        assert_eq!(kind.capability(), None);
        assert_eq!(kind.name(), "get_profile");

        // Custom is not dispatchable via resolve_kind (no capability filter).
        let dispatcher = CapabilityDispatcher::new();
        let tools = vec![make_tool("get_profile")];
        dispatcher.build_from_tools("memory.cpersona", &tools).await;
        assert!(dispatcher.resolve_kind(&kind).await.is_none());
    }

    #[tokio::test]
    async fn test_kind_capability_mapping_consistent_with_classify_tool() {
        // Every well-known ToolKind's tool name must be classified into the
        // same CapabilityType by the legacy classify_tool fallback. This is the
        // regression guard for manifest-vs-fallback divergence.
        for kind in well_known_kinds() {
            let cap = kind.capability().expect("well-known kind has capability");
            // Use a neutral server_id ("custom.x") so classify_tool falls back
            // to its tool-name match table rather than the server-prefix rule.
            let classified = classify_tool("custom.x", kind.name());
            assert_eq!(
                classified,
                Some(cap),
                "classify_tool divergence for kind={kind:?}: \
                 ToolKind.capability()={cap:?}, classify_tool({:?})={classified:?}",
                kind.name(),
            );
        }
    }

    #[test]
    fn test_capability_type_display_round_trip() {
        use std::str::FromStr;
        for cap in [
            CapabilityType::Memory,
            CapabilityType::Reasoning,
            CapabilityType::Vision,
            CapabilityType::Stt,
            CapabilityType::Speech,
        ] {
            let s = cap.to_string();
            let recovered = CapabilityType::from_str(&s).expect("FromStr should succeed");
            assert_eq!(recovered, cap);
        }
        assert!(CapabilityType::from_str("Unknown").is_err());
    }

    #[tokio::test]
    async fn test_resolve_kind_rejects_cross_capability() {
        // A server prefixed memory.* with the wrong tool name should not be
        // reachable via a different-capability ToolKind. (Defense in depth:
        // resolve() already filters by CapabilityType.)
        let dispatcher = CapabilityDispatcher::new();
        let tools = vec![make_tool("store")];
        dispatcher.build_from_tools("memory.cpersona", &tools).await;

        // Recall under Memory: found. Think under Reasoning: nothing registered.
        assert!(dispatcher.resolve_kind(&ToolKind::Store).await.is_some());
        assert!(dispatcher.resolve_kind(&ToolKind::Think).await.is_none());
    }
}
