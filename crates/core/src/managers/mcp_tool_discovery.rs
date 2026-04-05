//! MGP Tier 4 — Dynamic Tool Discovery & Active Tool Request (§16).
//!
//! Provides a searchable tool index, per-agent session tool cache with context
//! budget enforcement, and four kernel tools: `mgp.tools.discover`,
//! `mgp.tools.request`, `mgp.tools.session`, `mgp.tools.session.evict`.

use super::mcp::McpClientManager;
use super::mcp_mgp::ToolSecurityMetadata;
use super::mcp_protocol::McpTool;
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;
use tracing::{debug, warn};

// ============================================================
// Latency Tier (§16 — Tool Cost Awareness)
// ============================================================

/// Latency tier for tool cost awareness.
///
/// Higher tiers indicate slower / more expensive tools. The relevance score
/// is multiplied by `score_multiplier()` so that cheap tools rank higher
/// when keyword relevance is otherwise equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LatencyTier {
    /// Fast, in-process or cached (multiplier 1.0)
    C,
    /// Moderate local I/O (multiplier 0.9)
    B,
    /// Network round-trip or moderate LLM call (multiplier 0.7)
    A,
    /// Heavy: image gen, deep research, transcription, vision (multiplier 0.5)
    S,
}

impl LatencyTier {
    fn score_multiplier(self) -> f64 {
        match self {
            Self::C => 1.0,
            Self::B => 0.9,
            Self::A => 0.7,
            Self::S => 0.5,
        }
    }
}

/// Classify a tool into a latency tier based on server suffix and tool name.
fn classify_latency_tier(server_id: &str, tool_name: &str) -> LatencyTier {
    let suffix = server_id.split('.').next_back().unwrap_or("");

    match (suffix, tool_name) {
        // Tier S — very expensive
        ("imagegen", "generate_image")
        | ("research", "deep_research")
        | ("stt", "transcribe")
        | ("capture", "analyze_image") => LatencyTier::S,

        // Tier A — network or moderate LLM
        ("websearch", "fetch_page")
        | ("cpersona", "update_profile" | "archive_episode" | "recall")
        | ("ollama", "think" | "think_with_tools") => LatencyTier::A,

        // Tier B — light network / local I/O / hallucination-prone
        (_, "list_models" | "switch_model")
        | ("agent_utils", "get_current_time")
        | ("websearch", "search_status")
        | ("gaze", "start_tracking" | "stop_tracking" | "get_tracker_status") => LatencyTier::B,

        // Tier C — everything else (fast / safe default)
        _ => LatencyTier::C,
    }
}

/// Fixed overhead tokens per tool schema (name, description, type wrappers).
const SCHEMA_OVERHEAD_TOKENS: usize = 100;

/// Default context budget.
const DEFAULT_MAX_TOKENS: usize = 8000;

// ============================================================
// Tool Index (§16.4)
// ============================================================

/// A single entry in the searchable tool index.
#[derive(Debug, Clone)]
pub(super) struct ToolIndexEntry {
    pub tool_id: String,
    pub server_id: String,
    pub name: String,
    pub description: String,
    pub categories: Vec<String>,
    pub keywords: Vec<String>,
    pub input_schema: Value,
    pub security: Option<ToolSecurityMetadata>,
    pub estimated_tokens: usize,
    pub latency_tier: LatencyTier,
}

/// Search filter for tool discovery.
#[derive(Debug, Clone, Default)]
pub(super) struct ToolSearchFilter {
    categories: Option<Vec<String>>,
    risk_level_max: Option<String>,
}

/// Searchable tool index across all connected servers.
pub(super) struct ToolIndex {
    entries: Mutex<Vec<ToolIndexEntry>>,
}

impl ToolIndex {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
        }
    }

    /// Add all tools from a newly connected server.
    pub fn add_server_tools(
        &self,
        server_id: &str,
        tools: &[McpTool],
        security_fn: impl Fn(&str) -> Option<ToolSecurityMetadata>,
    ) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| {
            warn!("ToolIndex mutex poisoned — recovering");
            e.into_inner()
        });
        for tool in tools {
            // Skip mgp.* namespace (reserved for kernel tools)
            if tool.name.starts_with("mgp.") {
                continue;
            }

            let description = tool.description.clone().unwrap_or_default();
            let keywords = extract_keywords(&tool.name, &description);
            let categories = extract_categories(server_id, &tool.name);
            let estimated_tokens = estimate_tokens(&tool.input_schema) + SCHEMA_OVERHEAD_TOKENS;
            let security = security_fn(&tool.name);
            let latency_tier = classify_latency_tier(server_id, &tool.name);

            entries.push(ToolIndexEntry {
                tool_id: format!("{}.{}", server_id, tool.name),
                server_id: server_id.to_string(),
                name: tool.name.clone(),
                description,
                categories,
                keywords,
                input_schema: tool.input_schema.clone(),
                security,
                estimated_tokens,
                latency_tier,
            });
        }
        debug!(server = %server_id, count = tools.len(), "Tool index updated");
    }

    /// Remove all tools for a disconnected server.
    pub fn remove_server_tools(&self, server_id: &str) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| {
            warn!("ToolIndex mutex poisoned — recovering");
            e.into_inner()
        });
        entries.retain(|e| e.server_id != server_id);
        debug!(server = %server_id, "Tool index entries removed");
    }

    /// Keyword search. Returns entries sorted by relevance_score descending.
    pub fn search_keyword(
        &self,
        query: &str,
        max_results: usize,
        filter: &ToolSearchFilter,
    ) -> Vec<(ToolIndexEntry, f64)> {
        let entries = self.entries.lock().unwrap_or_else(|e| {
            warn!("ToolIndex mutex poisoned — recovering");
            e.into_inner()
        });
        let query_lower = query.to_lowercase();
        let query_tokens: Vec<&str> = query_lower.split_whitespace().collect();

        if query_tokens.is_empty() {
            return Vec::new();
        }

        let mut scored: Vec<(ToolIndexEntry, f64)> = entries
            .iter()
            .filter(|e| Self::matches_filter(e, filter))
            .filter_map(|entry| {
                let score = compute_relevance_score(entry, &query_tokens);
                if score > 0.0 {
                    Some((entry.clone(), score))
                } else {
                    None
                }
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(max_results);
        scored
    }

    /// Category filter. Returns entries matching any of the given categories.
    pub fn search_category(
        &self,
        categories: &[String],
        max_results: usize,
    ) -> Vec<(ToolIndexEntry, f64)> {
        let entries = self.entries.lock().unwrap_or_else(|e| {
            warn!("ToolIndex mutex poisoned — recovering");
            e.into_inner()
        });
        let cats_lower: Vec<String> = categories.iter().map(|c| c.to_lowercase()).collect();

        let mut results: Vec<(ToolIndexEntry, f64)> = entries
            .iter()
            .filter(|e| {
                e.categories
                    .iter()
                    .any(|c| cats_lower.contains(&c.to_lowercase()))
            })
            .map(|e| (e.clone(), 1.0))
            .collect();

        results.truncate(max_results);
        results
    }

    /// Total number of indexed tools.
    pub fn total_count(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(|e| {
                warn!("ToolIndex mutex poisoned — recovering");
                e.into_inner()
            })
            .len()
    }

    fn matches_filter(entry: &ToolIndexEntry, filter: &ToolSearchFilter) -> bool {
        if let Some(ref cats) = filter.categories {
            let cats_lower: Vec<String> = cats.iter().map(|c| c.to_lowercase()).collect();
            if !entry
                .categories
                .iter()
                .any(|c| cats_lower.contains(&c.to_lowercase()))
            {
                return false;
            }
        }
        if let Some(ref max_risk) = filter.risk_level_max {
            if let Some(ref sec) = entry.security {
                let risk_str = format!("{:?}", sec.effective_risk_level).to_lowercase();
                if !risk_within_max(&risk_str, max_risk) {
                    return false;
                }
            }
        }
        true
    }
}

/// Compute relevance score for an entry against query tokens.
fn compute_relevance_score(entry: &ToolIndexEntry, query_tokens: &[&str]) -> f64 {
    let name_lower = entry.name.to_lowercase();
    let desc_lower = entry.description.to_lowercase();
    let mut total_score = 0.0;

    for token in query_tokens {
        // Name exact match (highest)
        if name_lower == *token || name_lower.contains(token) {
            total_score += 1.0;
        }
        // Description word match
        else if desc_lower.contains(token) {
            total_score += 0.5;
        }
        // Keywords match
        else if entry.keywords.iter().any(|k| k == token) {
            total_score += 0.3;
        }
        // Category match
        else if entry
            .categories
            .iter()
            .any(|c| c.to_lowercase().contains(token))
        {
            total_score += 0.2;
        }
    }

    // Normalize by query token count, then apply latency tier penalty
    let normalized = total_score / query_tokens.len() as f64;
    normalized * entry.latency_tier.score_multiplier()
}

/// Check if a risk level is within the allowed maximum.
fn risk_within_max(risk: &str, max: &str) -> bool {
    let level = |s: &str| match s {
        "safe" => 0,
        "moderate" => 1,
        "dangerous" => 2,
        _ => 3,
    };
    level(risk) <= level(max)
}

/// Extract keywords from tool name and description.
fn extract_keywords(name: &str, description: &str) -> Vec<String> {
    let mut keywords = Vec::new();

    // Split name by underscores and dots
    for part in name.split(['_', '.']) {
        let lower = part.to_lowercase();
        if lower.len() > 1 {
            keywords.push(lower);
        }
    }

    // Extract significant words from description (>3 chars, lowercase)
    for word in description.split_whitespace() {
        let clean: String = word
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
            .to_lowercase();
        if clean.len() > 3 && !keywords.contains(&clean) {
            keywords.push(clean);
        }
    }

    keywords
}

/// Extract categories from server ID and tool name.
fn extract_categories(server_id: &str, tool_name: &str) -> Vec<String> {
    let mut categories = Vec::new();

    // Server prefix as category (e.g., "tool.terminal" → "terminal")
    if let Some(suffix) = server_id.split('.').next_back() {
        categories.push(suffix.to_lowercase());
    }

    // First part of tool name as category (e.g., "read_file" → "read")
    if let Some(prefix) = tool_name.split('_').next() {
        let lower = prefix.to_lowercase();
        if !categories.contains(&lower) {
            categories.push(lower);
        }
    }

    categories
}

/// Estimate token count from a JSON schema.
fn estimate_tokens(schema: &Value) -> usize {
    let json_str = serde_json::to_string(schema).unwrap_or_default();
    json_str.len() / 4
}

// ============================================================
// Session Tool Cache (§16.7)
// ============================================================

#[derive(Debug, Clone)]
struct CachedTool {
    last_used: Instant,
    estimated_tokens: usize,
}

/// Per-agent session state for the session tool cache.
struct AgentSession {
    pinned: Vec<String>,
    cached: HashMap<String, CachedTool>,
    max_tokens: usize,
}

/// Public session state returned by queries.
pub(super) struct SessionState {
    pub pinned: Vec<String>,
    pub cached: Vec<String>,
    pub total_tokens: usize,
    pub max_tokens: usize,
}

/// Per-agent session tool cache with LRU eviction and context budget.
pub(super) struct SessionToolCache {
    sessions: Mutex<HashMap<String, AgentSession>>,
}

impl SessionToolCache {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// Add tools to the session cache. Returns (tools_added, tokens_added).
    pub fn cache_tools(&self, agent_id: &str, tools: &[(String, usize)]) -> (usize, usize) {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| {
            warn!("SessionToolCache mutex poisoned — recovering");
            e.into_inner()
        });
        let session = sessions
            .entry(agent_id.to_string())
            .or_insert_with(|| AgentSession {
                pinned: Vec::new(),
                cached: HashMap::new(),
                max_tokens: DEFAULT_MAX_TOKENS,
            });

        let mut added = 0;
        let mut tokens_added = 0;

        for (tool_id, est_tokens) in tools {
            if session.cached.contains_key(tool_id) {
                // Already cached — just touch
                if let Some(entry) = session.cached.get_mut(tool_id) {
                    entry.last_used = Instant::now();
                }
                continue;
            }
            session.cached.insert(
                tool_id.clone(),
                CachedTool {
                    last_used: Instant::now(),
                    estimated_tokens: *est_tokens,
                },
            );
            added += 1;
            tokens_added += est_tokens;
        }

        Self::enforce_budget(session);

        (added, tokens_added)
    }

    /// Evict specified tools from cache. Returns count evicted.
    pub fn evict(&self, agent_id: &str, tool_ids: &[String]) -> usize {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| {
            warn!("SessionToolCache mutex poisoned — recovering");
            e.into_inner()
        });
        let Some(session) = sessions.get_mut(agent_id) else {
            return 0;
        };

        let mut evicted = 0;
        for tool_id in tool_ids {
            // Cannot evict pinned tools
            if session.pinned.contains(tool_id) {
                continue;
            }
            if session.cached.remove(tool_id).is_some() {
                evicted += 1;
            }
        }
        evicted
    }

    /// Get session state for query.
    pub fn get_session_state(&self, agent_id: &str) -> Option<SessionState> {
        let sessions = self.sessions.lock().unwrap_or_else(|e| {
            warn!("SessionToolCache mutex poisoned — recovering");
            e.into_inner()
        });
        sessions.get(agent_id).map(|session| {
            let total_tokens: usize = session.cached.values().map(|c| c.estimated_tokens).sum();
            SessionState {
                pinned: session.pinned.clone(),
                cached: session.cached.keys().cloned().collect(),
                total_tokens,
                max_tokens: session.max_tokens,
            }
        })
    }

    /// Set pinned tools for an agent.
    pub fn set_pinned(&self, agent_id: &str, tool_ids: Vec<String>) {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| {
            warn!("SessionToolCache mutex poisoned — recovering");
            e.into_inner()
        });
        let session = sessions
            .entry(agent_id.to_string())
            .or_insert_with(|| AgentSession {
                pinned: Vec::new(),
                cached: HashMap::new(),
                max_tokens: DEFAULT_MAX_TOKENS,
            });
        session.pinned = tool_ids;
    }

    /// Touch a tool (update last_used for LRU).
    pub fn touch(&self, agent_id: &str, tool_id: &str) {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| {
            warn!("SessionToolCache mutex poisoned — recovering");
            e.into_inner()
        });
        if let Some(session) = sessions.get_mut(agent_id) {
            if let Some(entry) = session.cached.get_mut(tool_id) {
                entry.last_used = Instant::now();
            }
        }
    }

    /// LRU eviction to fit within budget.
    fn enforce_budget(session: &mut AgentSession) {
        loop {
            let total: usize = session.cached.values().map(|c| c.estimated_tokens).sum();
            if total <= session.max_tokens {
                break;
            }
            // Find the least recently used non-pinned entry
            let lru_key = session
                .cached
                .iter()
                .filter(|(k, _)| !session.pinned.contains(k))
                .min_by_key(|(_, v)| v.last_used)
                .map(|(k, _)| k.clone());

            match lru_key {
                Some(key) => {
                    session.cached.remove(&key);
                }
                None => break, // All remaining are pinned
            }
        }
    }
}

// ============================================================
// Kernel Tool Schemas (§16)
// ============================================================

/// Schema for mgp.tools.discover (public for llm_meta_tool_schemas).
pub(super) fn tools_discover_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "mgp.tools.discover",
            "description": "Search for tools based on natural language description. Returns full tool schemas for immediate use.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Natural language description of needed capability"
                    },
                    "strategy": {
                        "type": "string",
                        "enum": ["keyword", "semantic", "category"],
                        "default": "keyword",
                        "description": "Search strategy"
                    },
                    "max_results": {
                        "type": "number",
                        "default": 5,
                        "description": "Maximum number of results to return"
                    },
                    "filter": {
                        "type": "object",
                        "properties": {
                            "categories": {
                                "type": "array",
                                "items": { "type": "string" }
                            },
                            "risk_level_max": {
                                "type": "string",
                                "enum": ["safe", "moderate", "dangerous"]
                            },
                            "status": {
                                "type": "string",
                                "enum": ["connected", "all"]
                            }
                        },
                        "description": "Filter criteria for results"
                    }
                },
                "required": ["query"]
            }
        }
    })
}

/// Schema for mgp.tools.request (public for llm_meta_tool_schemas).
pub(super) fn tools_request_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "mgp.tools.request",
            "description": "Request tools to fill a capability gap during task execution. The kernel searches for matching tools and loads them into your session.",
            "parameters": {
                "type": "object",
                "properties": {
                    "reason": {
                        "type": "string",
                        "enum": ["capability_gap", "performance", "preference"],
                        "description": "Reason for the tool request"
                    },
                    "context": {
                        "type": "string",
                        "description": "Why the tool is needed"
                    },
                    "requirements": {
                        "type": "object",
                        "properties": {
                            "capabilities": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Required capabilities (used as search keywords)"
                            },
                            "input_types": {
                                "type": "array",
                                "items": { "type": "string" }
                            },
                            "output_types": {
                                "type": "array",
                                "items": { "type": "string" }
                            },
                            "preferred_risk_level": {
                                "type": "string",
                                "enum": ["safe", "moderate", "dangerous"]
                            }
                        },
                        "description": "Tool requirements specification"
                    },
                    "task_trace_id": {
                        "type": "string",
                        "description": "Trace ID for audit"
                    }
                },
                "required": ["reason", "context", "requirements"]
            }
        }
    })
}

pub(super) fn tools_session_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "mgp.tools.session",
            "description": "Query the current session's loaded tools and context budget usage.",
            "parameters": {
                "type": "object",
                "properties": {
                    "pinned": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Set pinned tools for this agent (exempt from LRU eviction)"
                    }
                }
            }
        }
    })
}

pub(super) fn tools_session_evict_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "mgp.tools.session.evict",
            "description": "Remove tools from the session cache to free context space. Pinned tools cannot be evicted.",
            "parameters": {
                "type": "object",
                "properties": {
                    "tools": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Tool names to evict from session cache"
                    },
                    "reason": {
                        "type": "string",
                        "description": "Reason for eviction"
                    }
                },
                "required": ["tools"]
            }
        }
    })
}

// ============================================================
// Kernel Tool Executors (§16)
// ============================================================

/// Execute mgp.tools.discover — search the tool index (Mode A, §16.5).
pub(super) async fn execute_tools_discover(
    manager: &McpClientManager,
    args: Value,
) -> Result<Value> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: query"))?;
    let strategy = args
        .get("strategy")
        .and_then(|v| v.as_str())
        .unwrap_or("keyword");
    let max_results = args
        .get("max_results")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(5) as usize;

    let filter = {
        let f = args.get("filter");
        ToolSearchFilter {
            categories: f
                .and_then(|v| v.get("categories"))
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                }),
            risk_level_max: f
                .and_then(|v| v.get("risk_level_max"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
        }
    };

    let start = Instant::now();

    let (results, actual_strategy, fallback) = match strategy {
        "category" => {
            let cats = filter.categories.clone().unwrap_or_default();
            if cats.is_empty() {
                return Err(anyhow::anyhow!(
                    "Category search requires filter.categories"
                ));
            }
            let r = manager.rich_tool_index.search_category(&cats, max_results);
            (r, "category", None)
        }
        "semantic" => {
            // Semantic search not implemented — fall back to keyword (§16.4)
            let r = manager
                .rich_tool_index
                .search_keyword(query, max_results, &filter);
            (r, "keyword", Some("keyword"))
        }
        _ => {
            let r = manager
                .rich_tool_index
                .search_keyword(query, max_results, &filter);
            (r, "keyword", None)
        }
    };

    let query_time_ms = start.elapsed().as_millis() as u64;
    let total_available = manager.rich_tool_index.total_count();

    let tools_json: Vec<Value> = results
        .iter()
        .map(|(entry, score)| {
            let mut tool = serde_json::json!({
                "name": entry.name,
                "server_id": entry.server_id,
                "description": entry.description,
                "relevance_score": (*score * 100.0).round() / 100.0,
                "inputSchema": entry.input_schema,
            });
            if let Some(ref sec) = entry.security {
                tool["security"] = serde_json::to_value(sec).unwrap_or_default();
            }
            tool
        })
        .collect();

    let mut response = serde_json::json!({
        "tools": tools_json,
        "total_available": total_available,
        "search_strategy": actual_strategy,
        "query_time_ms": query_time_ms,
    });

    if let Some(fb) = fallback {
        response["fallback_strategy"] = serde_json::json!(fb);
    }

    debug!(query = %query, strategy = %actual_strategy, results = tools_json.len(), "Tool discovery completed");
    Ok(response)
}

/// Execute mgp.tools.request — active tool request (Mode B, §16.6).
pub(super) async fn execute_tools_request(
    manager: &McpClientManager,
    args: Value,
) -> Result<Value> {
    let _reason = args
        .get("reason")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: reason"))?;
    let _context = args
        .get("context")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: context"))?;
    let requirements = args
        .get("requirements")
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: requirements"))?;

    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    // Extract capabilities as search keywords
    let capabilities: Vec<String> = requirements
        .get("capabilities")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let preferred_risk = requirements
        .get("preferred_risk_level")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    // Search for matching tools using capabilities as query
    let query = capabilities.join(" ");
    let filter = ToolSearchFilter {
        categories: None,
        risk_level_max: preferred_risk,
    };

    let results = manager.rich_tool_index.search_keyword(&query, 10, &filter);

    if results.is_empty() {
        return Ok(serde_json::json!({
            "status": "unavailable",
            "tools_loaded": [],
            "tools_unavailable": capabilities,
            "session_tools_count": 0,
            "context_tokens_added": 0,
        }));
    }

    // Add found tools to session cache
    let cache_entries: Vec<(String, usize)> = results
        .iter()
        .map(|(entry, _)| (entry.tool_id.clone(), entry.estimated_tokens))
        .collect();

    let (added, tokens_added) = manager.session_cache.cache_tools(agent_id, &cache_entries);

    let tools_loaded: Vec<Value> = results
        .iter()
        .map(|(entry, _)| {
            let mut tool = serde_json::json!({
                "name": entry.name,
                "server_id": entry.server_id,
                "description": entry.description,
                "inputSchema": entry.input_schema,
            });
            if let Some(ref sec) = entry.security {
                tool["security"] = serde_json::to_value(sec).unwrap_or_default();
            }
            tool
        })
        .collect();

    let session_state = manager.session_cache.get_session_state(agent_id);
    let session_tools_count = session_state.as_ref().map_or(0, |s| s.cached.len());

    let status = if added == results.len() {
        "fulfilled"
    } else if added > 0 {
        "partial"
    } else {
        "fulfilled" // All were already cached
    };

    debug!(
        agent = %agent_id,
        status = %status,
        tools_loaded = added,
        "Tool request completed"
    );

    Ok(serde_json::json!({
        "status": status,
        "tools_loaded": tools_loaded,
        "tools_unavailable": [],
        "session_tools_count": session_tools_count,
        "context_tokens_added": tokens_added,
    }))
}

/// Execute mgp.tools.session — query session state (§16.7).
pub(super) async fn execute_tools_session(
    manager: &McpClientManager,
    args: Value,
) -> Result<Value> {
    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    // Handle pinned tool update if provided (§16.7)
    if let Some(pinned_arr) = args.get("pinned").and_then(|v| v.as_array()) {
        let pinned: Vec<String> = pinned_arr
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        manager.session_cache.set_pinned(agent_id, pinned);
    }

    let state = manager.session_cache.get_session_state(agent_id);

    match state {
        Some(s) => Ok(serde_json::json!({
            "pinned": s.pinned,
            "cached": s.cached,
            "total_tokens": s.total_tokens,
            "max_tokens": s.max_tokens,
        })),
        None => Ok(serde_json::json!({
            "pinned": [],
            "cached": [],
            "total_tokens": 0,
            "max_tokens": DEFAULT_MAX_TOKENS,
        })),
    }
}

/// Execute mgp.tools.session.evict — remove tools from cache (§16.7).
pub(super) async fn execute_tools_session_evict(
    manager: &McpClientManager,
    args: Value,
) -> Result<Value> {
    let tools: Vec<String> = args
        .get("tools")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: tools"))?;

    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let evicted = manager.session_cache.evict(agent_id, &tools);

    let state = manager.session_cache.get_session_state(agent_id);
    let remaining = state.as_ref().map_or(0, |s| s.cached.len());
    let freed = evicted * SCHEMA_OVERHEAD_TOKENS; // Approximate

    debug!(agent = %agent_id, evicted = evicted, "Session tools evicted");

    Ok(serde_json::json!({
        "evicted": evicted,
        "remaining_count": remaining,
        "freed_tokens": freed,
    }))
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool(name: &str, desc: &str) -> McpTool {
        McpTool {
            name: name.to_string(),
            description: Some(desc.to_string()),
            input_schema: serde_json::json!({"type": "object", "properties": {}}),
            annotations: None,
        }
    }

    #[test]
    fn tool_index_add_and_search() {
        let index = ToolIndex::new();
        let tools = vec![
            make_tool("read_file", "Read the contents of a file"),
            make_tool("write_file", "Write content to a file"),
            make_tool("grep", "Search file contents using pattern matching"),
        ];
        index.add_server_tools("tool.terminal", &tools, |_| None);

        let results = index.search_keyword("read file", 5, &ToolSearchFilter::default());
        assert!(!results.is_empty());
        assert_eq!(results[0].0.name, "read_file");
    }

    #[test]
    fn tool_index_remove_server() {
        let index = ToolIndex::new();
        let tools = vec![make_tool("test_tool", "A test tool")];
        index.add_server_tools("srv1", &tools, |_| None);
        assert_eq!(index.total_count(), 1);

        index.remove_server_tools("srv1");
        assert_eq!(index.total_count(), 0);
    }

    #[test]
    fn tool_index_category_search() {
        let index = ToolIndex::new();
        let tools = vec![
            make_tool("read_file", "Read a file"),
            make_tool("execute_command", "Run a shell command"),
        ];
        index.add_server_tools("tool.terminal", &tools, |_| None);

        let results = index.search_category(&["terminal".to_string()], 10);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn tool_index_max_results() {
        let index = ToolIndex::new();
        let tools: Vec<McpTool> = (0..10)
            .map(|i| make_tool(&format!("tool_{i}"), &format!("Tool number {i}")))
            .collect();
        index.add_server_tools("srv", &tools, |_| None);

        let results = index.search_keyword("tool", 3, &ToolSearchFilter::default());
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn tool_index_keyword_scoring() {
        let index = ToolIndex::new();
        let tools = vec![
            make_tool("analyze", "Analyze data"),
            make_tool("read_file", "Read and analyze file contents"),
        ];
        index.add_server_tools("srv", &tools, |_| None);

        let results = index.search_keyword("analyze", 5, &ToolSearchFilter::default());
        assert!(!results.is_empty());
        // "analyze" should score higher (name match) than "read_file" (description match)
        assert_eq!(results[0].0.name, "analyze");
    }

    #[test]
    fn tool_index_empty_returns_empty() {
        let index = ToolIndex::new();
        let tools = vec![make_tool("test", "A test")];
        index.add_server_tools("srv", &tools, |_| None);

        let results = index.search_keyword("", 5, &ToolSearchFilter::default());
        assert!(results.is_empty());
    }

    #[test]
    fn session_cache_add_and_query() {
        let cache = SessionToolCache::new();
        let tools = vec![
            ("srv.read_file".to_string(), 200_usize),
            ("srv.write_file".to_string(), 180),
        ];
        let (added, tokens) = cache.cache_tools("agent1", &tools);
        assert_eq!(added, 2);
        assert_eq!(tokens, 380);

        let state = cache.get_session_state("agent1").unwrap();
        assert_eq!(state.cached.len(), 2);
        assert_eq!(state.total_tokens, 380);
    }

    #[test]
    fn session_cache_evict() {
        let cache = SessionToolCache::new();
        cache.cache_tools("agent1", &[("srv.tool1".to_string(), 200)]);
        let evicted = cache.evict("agent1", &["srv.tool1".to_string()]);
        assert_eq!(evicted, 1);

        let state = cache.get_session_state("agent1").unwrap();
        assert!(state.cached.is_empty());
    }

    #[test]
    fn session_cache_pinned_not_evictable() {
        let cache = SessionToolCache::new();
        cache.set_pinned("agent1", vec!["srv.pinned_tool".to_string()]);
        cache.cache_tools("agent1", &[("srv.pinned_tool".to_string(), 200)]);

        let evicted = cache.evict("agent1", &["srv.pinned_tool".to_string()]);
        assert_eq!(evicted, 0);

        let state = cache.get_session_state("agent1").unwrap();
        assert_eq!(state.cached.len(), 1);
    }

    #[test]
    fn session_cache_lru_eviction() {
        let cache = SessionToolCache::new();
        // Set a small budget
        {
            let mut sessions = cache.sessions.lock().unwrap();
            sessions.insert(
                "agent1".to_string(),
                AgentSession {
                    pinned: Vec::new(),
                    cached: HashMap::new(),
                    max_tokens: 500,
                },
            );
        }

        // Add tools that exceed budget
        cache.cache_tools("agent1", &[("tool1".to_string(), 300)]);
        std::thread::sleep(std::time::Duration::from_millis(10));
        cache.cache_tools("agent1", &[("tool2".to_string(), 300)]);

        // Budget is 500, total would be 600 — LRU (tool1) should be evicted
        let state = cache.get_session_state("agent1").unwrap();
        assert_eq!(state.cached.len(), 1);
        assert!(state.cached.contains(&"tool2".to_string()));
    }

    #[test]
    fn session_cache_per_agent() {
        let cache = SessionToolCache::new();
        cache.cache_tools("agent1", &[("tool_a".to_string(), 100)]);
        cache.cache_tools("agent2", &[("tool_b".to_string(), 100)]);

        let s1 = cache.get_session_state("agent1").unwrap();
        let s2 = cache.get_session_state("agent2").unwrap();
        assert_eq!(s1.cached.len(), 1);
        assert_eq!(s2.cached.len(), 1);
        assert!(s1.cached.contains(&"tool_a".to_string()));
        assert!(s2.cached.contains(&"tool_b".to_string()));
    }

    // ============================================================
    // Stress / Load Tests
    // ============================================================

    /// Stress test: 200 tools registered, search performance and correctness.
    #[test]
    fn stress_tool_index_200_tools() {
        let index = ToolIndex::new();
        let tools: Vec<McpTool> = (0..200)
            .map(|i| {
                make_tool(
                    &format!("tool_{i}"),
                    &format!("Tool number {i} for processing data batch {}", i % 10),
                )
            })
            .collect();
        index.add_server_tools("srv.stress", &tools, |_| None);
        assert_eq!(index.total_count(), 200);

        // Keyword search still returns bounded results
        let results = index.search_keyword("processing data", 10, &ToolSearchFilter::default());
        assert!(results.len() <= 10);
        assert!(!results.is_empty());

        // All results have positive relevance scores
        for (_, score) in &results {
            assert!(*score > 0.0);
        }
    }

    /// Stress test: 500 tools across 10 servers.
    #[test]
    fn stress_tool_index_multi_server() {
        let index = ToolIndex::new();
        for srv in 0..10 {
            let tools: Vec<McpTool> = (0..50)
                .map(|i| {
                    make_tool(
                        &format!("action_{i}"),
                        &format!("Perform action {i} on server {srv}"),
                    )
                })
                .collect();
            index.add_server_tools(&format!("srv.batch{srv}"), &tools, |_| None);
        }
        assert_eq!(index.total_count(), 500);

        // Category search by server prefix works
        let results = index.search_category(&["batch3".to_string()], 100);
        assert_eq!(results.len(), 50);

        // Removing one server leaves the rest intact
        index.remove_server_tools("srv.batch0");
        assert_eq!(index.total_count(), 450);
    }

    /// Budget boundary: tools that exactly fill the budget.
    #[test]
    fn stress_budget_exact_fill() {
        let cache = SessionToolCache::new();
        {
            let mut sessions = cache.sessions.lock().unwrap();
            sessions.insert(
                "agent_exact".to_string(),
                AgentSession {
                    pinned: Vec::new(),
                    cached: HashMap::new(),
                    max_tokens: 1000,
                },
            );
        }

        // Add exactly 1000 tokens (10 tools × 100 tokens)
        let tools: Vec<(String, usize)> = (0..10).map(|i| (format!("t{i}"), 100)).collect();
        cache.cache_tools("agent_exact", &tools);

        let state = cache.get_session_state("agent_exact").unwrap();
        assert_eq!(state.cached.len(), 10);
        assert_eq!(state.total_tokens, 1000);
    }

    /// Budget boundary: one token over budget triggers LRU eviction.
    #[test]
    fn stress_budget_one_over() {
        let cache = SessionToolCache::new();
        {
            let mut sessions = cache.sessions.lock().unwrap();
            sessions.insert(
                "agent_over".to_string(),
                AgentSession {
                    pinned: Vec::new(),
                    cached: HashMap::new(),
                    max_tokens: 1000,
                },
            );
        }

        // Fill to 900
        let tools: Vec<(String, usize)> = (0..9).map(|i| (format!("t{i}"), 100)).collect();
        cache.cache_tools("agent_over", &tools);

        // Wait to establish LRU ordering
        std::thread::sleep(std::time::Duration::from_millis(10));

        // Add a 200-token tool — total would be 1100 > 1000, so LRU eviction
        cache.cache_tools("agent_over", &[("t_big".to_string(), 200)]);

        let state = cache.get_session_state("agent_over").unwrap();
        assert!(
            state.total_tokens <= 1000,
            "Budget must not be exceeded: {}",
            state.total_tokens
        );
        // The big tool should be present
        assert!(state.cached.contains(&"t_big".to_string()));
        // At least one old tool was evicted
        assert!(state.cached.len() < 10);
    }

    /// Stress test: 50 concurrent agents each with independent budgets.
    #[test]
    fn stress_concurrent_agents_independence() {
        let cache = SessionToolCache::new();

        for a in 0..50 {
            let agent_id = format!("agent_{a}");
            let tools: Vec<(String, usize)> = (0..5)
                .map(|i| (format!("{agent_id}.tool_{i}"), 100))
                .collect();
            cache.cache_tools(&agent_id, &tools);
        }

        // Each agent should have exactly 5 tools, 500 tokens
        for a in 0..50 {
            let state = cache.get_session_state(&format!("agent_{a}")).unwrap();
            assert_eq!(state.cached.len(), 5);
            assert_eq!(state.total_tokens, 500);
        }
    }

    /// Stress test: LRU eviction with many insertions preserves most recent.
    #[test]
    fn stress_lru_eviction_ordering() {
        let cache = SessionToolCache::new();
        {
            let mut sessions = cache.sessions.lock().unwrap();
            sessions.insert(
                "agent_lru".to_string(),
                AgentSession {
                    pinned: Vec::new(),
                    cached: HashMap::new(),
                    max_tokens: 500,
                },
            );
        }

        // Insert 20 tools sequentially (200 tokens each, budget 500)
        // Only last 2 should survive (200 × 2 = 400 ≤ 500)
        for i in 0..20 {
            cache.cache_tools("agent_lru", &[(format!("tool_{i}"), 200)]);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let state = cache.get_session_state("agent_lru").unwrap();
        assert!(state.total_tokens <= 500);
        // Most recent tool must be present
        assert!(
            state.cached.contains(&"tool_19".to_string()),
            "Most recent tool must survive LRU eviction"
        );
    }

    /// Stress test: pinned tools survive LRU eviction under budget pressure.
    #[test]
    fn stress_pinned_survives_budget_pressure() {
        let cache = SessionToolCache::new();
        {
            let mut sessions = cache.sessions.lock().unwrap();
            sessions.insert(
                "agent_pin".to_string(),
                AgentSession {
                    pinned: vec!["pinned_tool".to_string()],
                    cached: HashMap::new(),
                    max_tokens: 500,
                },
            );
        }

        // Add pinned tool first
        cache.cache_tools("agent_pin", &[("pinned_tool".to_string(), 200)]);

        // Fill remaining budget and exceed it
        for i in 0..10 {
            cache.cache_tools("agent_pin", &[(format!("tool_{i}"), 200)]);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let state = cache.get_session_state("agent_pin").unwrap();
        assert!(state.total_tokens <= 500);
        // Pinned tool must survive all evictions
        assert!(
            state.cached.contains(&"pinned_tool".to_string()),
            "Pinned tool must never be evicted"
        );
    }

    /// Stress test: large schemas with realistic token estimates.
    #[test]
    fn stress_large_schema_token_estimation() {
        // Build a tool with a large input_schema
        let mut properties = serde_json::Map::new();
        for i in 0..50 {
            properties.insert(
                format!("param_{i}"),
                serde_json::json!({
                    "type": "string",
                    "description": format!("Parameter number {i} with a moderately long description text")
                }),
            );
        }
        let schema = serde_json::json!({
            "type": "object",
            "properties": properties,
        });

        let estimated = estimate_tokens(&schema);
        // 50 properties × ~80 chars each ≈ 4000 chars ÷ 4 ≈ 1000 tokens + overhead
        assert!(
            estimated > 500,
            "Large schema should estimate > 500 tokens, got {estimated}"
        );
        assert!(
            estimated < 5000,
            "Estimate should be reasonable, got {estimated}"
        );
    }

    /// Stress test: duplicate tool insertion is idempotent (touch only).
    #[test]
    fn stress_duplicate_insertion_idempotent() {
        let cache = SessionToolCache::new();

        // Add same tool 100 times
        for _ in 0..100 {
            cache.cache_tools("agent_dup", &[("same_tool".to_string(), 200)]);
        }

        let state = cache.get_session_state("agent_dup").unwrap();
        assert_eq!(
            state.cached.len(),
            1,
            "Duplicate adds must not create extra entries"
        );
        assert_eq!(
            state.total_tokens, 200,
            "Token count must not grow from duplicates"
        );
    }

    /// Stress test: mass eviction returns correct count.
    #[test]
    fn stress_mass_eviction() {
        let cache = SessionToolCache::new();
        let tools: Vec<(String, usize)> = (0..100).map(|i| (format!("tool_{i}"), 50)).collect();
        cache.cache_tools("agent_mass", &tools);

        // Evict all
        let tool_ids: Vec<String> = (0..100).map(|i| format!("tool_{i}")).collect();
        let evicted = cache.evict("agent_mass", &tool_ids);
        assert_eq!(evicted, 100);

        let state = cache.get_session_state("agent_mass").unwrap();
        assert!(state.cached.is_empty());
        assert_eq!(state.total_tokens, 0);
    }

    /// Stress test: evict non-existent tools returns 0.
    #[test]
    fn stress_evict_nonexistent() {
        let cache = SessionToolCache::new();
        cache.cache_tools("agent_ne", &[("real_tool".to_string(), 100)]);

        let evicted = cache.evict("agent_ne", &["fake_tool".to_string()]);
        assert_eq!(evicted, 0);

        // Original tool untouched
        let state = cache.get_session_state("agent_ne").unwrap();
        assert_eq!(state.cached.len(), 1);
    }

    /// Edge case: empty query on non-empty index.
    #[test]
    fn stress_empty_query_on_large_index() {
        let index = ToolIndex::new();
        let tools: Vec<McpTool> = (0..100)
            .map(|i| make_tool(&format!("tool_{i}"), &format!("Description {i}")))
            .collect();
        index.add_server_tools("srv", &tools, |_| None);

        let results = index.search_keyword("", 50, &ToolSearchFilter::default());
        assert!(results.is_empty(), "Empty query must return no results");
    }

    /// Edge case: search with no matching terms on large index.
    #[test]
    fn stress_no_match_on_large_index() {
        let index = ToolIndex::new();
        let tools: Vec<McpTool> = (0..100)
            .map(|i| make_tool(&format!("tool_{i}"), &format!("Description {i}")))
            .collect();
        index.add_server_tools("srv", &tools, |_| None);

        let results = index.search_keyword("zzzzzyyyy", 50, &ToolSearchFilter::default());
        assert!(
            results.is_empty(),
            "Non-matching query must return no results"
        );
    }

    // ============================================================
    // Context Reduction Measurement Tests
    // ============================================================

    /// Helper: create a realistic tool schema with N parameters.
    fn make_realistic_tool(name: &str, desc: &str, param_count: usize) -> McpTool {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        for i in 0..param_count {
            properties.insert(
                format!("param_{i}"),
                serde_json::json!({
                    "type": if i % 3 == 0 { "string" } else if i % 3 == 1 { "number" } else { "boolean" },
                    "description": format!("Parameter {i} for {name}: controls behavior aspect {i}")
                }),
            );
            if i < param_count / 2 {
                required.push(serde_json::Value::String(format!("param_{i}")));
            }
        }
        McpTool {
            name: name.to_string(),
            description: Some(desc.to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": properties,
                "required": required,
            }),
            annotations: None,
        }
    }

    /// Measure context reduction: realistic MCP server deployment.
    ///
    /// Simulates a typical ClotoCore setup with multiple MCP servers
    /// and measures the token savings when using session cache vs full injection.
    #[test]
    #[allow(clippy::too_many_lines)]
    fn measure_context_reduction_realistic() {
        let index = ToolIndex::new();

        // Simulate realistic MCP server tool distribution:
        // terminal: 5 tools (3-5 params each)
        // agent_utils: 8 tools (2-4 params)
        // cron: 5 tools (2-3 params)
        // embedding: 3 tools (1-2 params)
        // websearch: 3 tools (2-4 params)
        // research: 2 tools (3-5 params)
        // capture: 2 tools (4-6 params)
        // imagegen: 3 tools (5-8 params)
        // stt: 2 tools (2-3 params)
        // tts: 3 tools (2-4 params)
        // gaze: 5 tools (3-6 params)
        #[allow(clippy::type_complexity)]
        let server_configs: Vec<(&str, Vec<(&str, &str, usize)>)> = vec![
            (
                "tool.terminal",
                vec![
                    (
                        "execute_command",
                        "Execute a shell command in the terminal",
                        5,
                    ),
                    (
                        "read_file",
                        "Read the contents of a file from the filesystem",
                        3,
                    ),
                    ("write_file", "Write content to a file on the filesystem", 4),
                    ("list_directory", "List files and directories in a path", 3),
                    (
                        "get_system_info",
                        "Get system information and resource usage",
                        2,
                    ),
                ],
            ),
            (
                "tool.agent_utils",
                vec![
                    (
                        "create_agent",
                        "Create a new agent with specified configuration",
                        4,
                    ),
                    ("update_agent", "Update an existing agent's settings", 3),
                    ("delete_agent", "Delete an agent by ID", 2),
                    ("list_agents", "List all registered agents", 2),
                    ("assign_plugin", "Assign a plugin to an agent", 3),
                    (
                        "unassign_plugin",
                        "Remove a plugin assignment from an agent",
                        3,
                    ),
                    ("get_agent_status", "Get the current status of an agent", 2),
                    ("set_agent_mode", "Set the operational mode of an agent", 3),
                ],
            ),
            (
                "tool.cron",
                vec![
                    ("create_schedule", "Create a new scheduled task", 5),
                    ("update_schedule", "Update an existing schedule", 4),
                    ("delete_schedule", "Delete a scheduled task", 2),
                    ("list_schedules", "List all scheduled tasks", 2),
                    ("trigger_schedule", "Manually trigger a scheduled task", 3),
                ],
            ),
            (
                "tool.embedding",
                vec![
                    ("embed_text", "Generate embeddings for text input", 2),
                    ("embed_batch", "Generate embeddings for multiple texts", 3),
                    (
                        "similarity_search",
                        "Find similar texts using embeddings",
                        4,
                    ),
                ],
            ),
            (
                "tool.websearch",
                vec![
                    ("search_web", "Search the web using a query string", 4),
                    ("fetch_url", "Fetch and extract content from a URL", 3),
                    ("search_news", "Search recent news articles", 3),
                ],
            ),
            (
                "tool.research",
                vec![
                    ("deep_research", "Conduct deep research on a topic", 5),
                    (
                        "summarize_sources",
                        "Summarize multiple research sources",
                        4,
                    ),
                ],
            ),
            (
                "tool.capture",
                vec![
                    ("screenshot", "Capture a screenshot of the screen", 4),
                    (
                        "screen_region",
                        "Capture a specific region of the screen",
                        6,
                    ),
                ],
            ),
            (
                "tool.imagegen",
                vec![
                    ("generate_image", "Generate an image from text prompt", 6),
                    ("edit_image", "Edit an existing image with instructions", 8),
                    ("upscale_image", "Upscale an image to higher resolution", 5),
                ],
            ),
            (
                "tool.stt",
                vec![
                    ("transcribe_audio", "Transcribe audio to text", 3),
                    (
                        "transcribe_stream",
                        "Transcribe streaming audio in real-time",
                        4,
                    ),
                ],
            ),
            (
                "tool.tts",
                vec![
                    ("synthesize_speech", "Convert text to speech audio", 4),
                    ("list_voices", "List available TTS voices", 2),
                    ("set_voice", "Set the default TTS voice", 3),
                ],
            ),
            (
                "tool.gaze",
                vec![
                    ("track_gaze", "Start eye gaze tracking", 3),
                    ("get_gaze_position", "Get current gaze position", 2),
                    ("calibrate", "Run gaze tracker calibration", 4),
                    ("set_sensitivity", "Adjust gaze tracking sensitivity", 3),
                    ("get_heatmap", "Generate a gaze heatmap for a session", 5),
                ],
            ),
        ];

        let mut total_tools = 0;
        for (server_id, tools_config) in &server_configs {
            let tools: Vec<McpTool> = tools_config
                .iter()
                .map(|(name, desc, params)| make_realistic_tool(name, desc, *params))
                .collect();
            total_tools += tools.len();
            index.add_server_tools(server_id, &tools, |_| None);
        }

        // Calculate full injection cost (all tools)
        let all_entries = {
            let entries = index.entries.lock().unwrap();
            entries.clone()
        };
        let full_injection_tokens: usize = all_entries.iter().map(|e| e.estimated_tokens).sum();

        // Simulate a typical task: agent needs terminal + websearch (8 tools out of 41)
        let task_results = index.search_keyword(
            "execute command file search web",
            8,
            &ToolSearchFilter::default(),
        );

        let session_tokens: usize = task_results.iter().map(|(e, _)| e.estimated_tokens).sum();

        let reduction_pct = if full_injection_tokens > 0 {
            ((full_injection_tokens - session_tokens) as f64 / full_injection_tokens as f64) * 100.0
        } else {
            0.0
        };

        // Print measurement results (visible in test output with --nocapture)
        eprintln!("\n=== Context Reduction Measurement ===");
        eprintln!(
            "Total tools across {} servers: {}",
            server_configs.len(),
            total_tools
        );
        eprintln!("Full injection tokens: {}", full_injection_tokens);
        eprintln!(
            "Session cache tokens (8 task-relevant tools): {}",
            session_tokens
        );
        eprintln!("Reduction: {:.1}%", reduction_pct);
        eprintln!(
            "Budget: {} tokens (session uses {:.1}% of budget)",
            DEFAULT_MAX_TOKENS,
            (session_tokens as f64 / DEFAULT_MAX_TOKENS as f64) * 100.0
        );
        eprintln!("=====================================\n");

        // Assertions: meaningful reduction must be achieved
        assert!(
            total_tools >= 40,
            "Realistic deployment should have 40+ tools, got {total_tools}"
        );
        assert!(
            reduction_pct >= 50.0,
            "Context reduction must be at least 50% for typical task, got {reduction_pct:.1}%"
        );
        assert!(
            session_tokens <= DEFAULT_MAX_TOKENS,
            "Session tokens ({session_tokens}) must fit within budget ({DEFAULT_MAX_TOKENS})"
        );
    }

    /// Measure context reduction: worst case (agent needs many tools).
    #[test]
    fn measure_context_reduction_heavy_task() {
        let index = ToolIndex::new();

        // 60 tools across 6 servers with varying complexity
        for srv in 0..6 {
            let tools: Vec<McpTool> = (0..10)
                .map(|i| {
                    let params = 2 + (i % 5); // 2-6 params
                    make_realistic_tool(
                        &format!("action_{i}"),
                        &format!("Perform action {i} involving data processing and transformation"),
                        params,
                    )
                })
                .collect();
            index.add_server_tools(&format!("tool.service{srv}"), &tools, |_| None);
        }

        let all_entries = {
            let entries = index.entries.lock().unwrap();
            entries.clone()
        };
        let full_tokens: usize = all_entries.iter().map(|e| e.estimated_tokens).sum();

        // Heavy task: agent wants 20 tools (worst case but still selective)
        let results =
            index.search_keyword("data processing action", 20, &ToolSearchFilter::default());
        let session_tokens: usize = results.iter().map(|(e, _)| e.estimated_tokens).sum();

        let reduction_pct = ((full_tokens - session_tokens) as f64 / full_tokens as f64) * 100.0;

        eprintln!("\n=== Context Reduction (Heavy Task) ===");
        eprintln!(
            "Total tools: {}, Full tokens: {}",
            index.total_count(),
            full_tokens
        );
        eprintln!(
            "Requested tools: {}, Session tokens: {}",
            results.len(),
            session_tokens
        );
        eprintln!("Reduction: {:.1}%", reduction_pct);
        eprintln!("=======================================\n");

        // Even heavy tasks should save something
        assert!(
            reduction_pct >= 20.0,
            "Even heavy tasks should achieve 20%+ reduction, got {reduction_pct:.1}%"
        );
    }

    /// Measure per-tool token overhead accuracy.
    #[test]
    fn measure_token_estimation_accuracy() {
        // Test with various schema complexities
        let test_cases: Vec<(&str, usize, usize, usize)> = vec![
            // (name, param_count, expected_min_tokens, expected_max_tokens)
            ("simple_tool", 1, 100, 300),
            ("medium_tool", 5, 150, 600),
            ("complex_tool", 10, 300, 1200),
            ("heavy_tool", 20, 600, 2500),
        ];

        eprintln!("\n=== Token Estimation Accuracy ===");
        for (name, params, min, max) in &test_cases {
            let tool =
                make_realistic_tool(name, &format!("A tool with {params} parameters"), *params);
            let tokens = estimate_tokens(&tool.input_schema) + SCHEMA_OVERHEAD_TOKENS;
            eprintln!("  {name} ({params} params): {tokens} tokens");
            assert!(
                tokens >= *min && tokens <= *max,
                "{name}: {tokens} tokens outside expected range [{min}, {max}]"
            );
        }
        eprintln!("=================================\n");
    }

    // ============================================================
    // Latency Tier Tests
    // ============================================================

    #[test]
    fn latency_tier_affects_scoring() {
        let index = ToolIndex::new();

        // Two tools with the keyword "search" — one Tier C (websearch), one Tier S (research)
        let fast_tool = make_tool("web_search", "Search the web for information");
        let slow_tool = make_tool(
            "deep_research",
            "Deep research search across multiple sources",
        );

        index.add_server_tools("tool.websearch", &[fast_tool], |_| None);
        index.add_server_tools("tool.research", &[slow_tool], |_| None);

        let results = index.search_keyword("search", 10, &ToolSearchFilter::default());
        assert!(results.len() >= 2, "Both tools should match 'search'");

        // web_search (Tier C, multiplier 1.0) should rank above deep_research (Tier S, multiplier 0.5)
        assert_eq!(
            results[0].0.name, "web_search",
            "Tier C tool should rank above Tier S tool for equal keyword relevance"
        );
        assert!(
            results[0].1 > results[1].1,
            "Tier C score ({}) should be higher than Tier S score ({})",
            results[0].1,
            results[1].1,
        );
    }

    #[test]
    fn classify_latency_tier_mapping() {
        // Tier S
        assert_eq!(
            classify_latency_tier("tool.imagegen", "generate_image"),
            LatencyTier::S
        );
        assert_eq!(
            classify_latency_tier("tool.research", "deep_research"),
            LatencyTier::S
        );
        assert_eq!(
            classify_latency_tier("tool.stt", "transcribe"),
            LatencyTier::S
        );
        assert_eq!(
            classify_latency_tier("tool.capture", "analyze_image"),
            LatencyTier::S
        );

        // Tier A
        assert_eq!(
            classify_latency_tier("tool.websearch", "fetch_page"),
            LatencyTier::A
        );
        assert_eq!(
            classify_latency_tier("tool.cpersona", "update_profile"),
            LatencyTier::A
        );
        assert_eq!(
            classify_latency_tier("tool.cpersona", "archive_episode"),
            LatencyTier::A
        );
        assert_eq!(
            classify_latency_tier("tool.cpersona", "recall"),
            LatencyTier::A
        );
        assert_eq!(
            classify_latency_tier("tool.ollama", "think"),
            LatencyTier::A
        );
        assert_eq!(
            classify_latency_tier("tool.ollama", "think_with_tools"),
            LatencyTier::A
        );

        // Tier B
        assert_eq!(
            classify_latency_tier("tool.cerebras", "list_models"),
            LatencyTier::B
        );
        assert_eq!(
            classify_latency_tier("tool.deepseek", "switch_model"),
            LatencyTier::B
        );
        assert_eq!(
            classify_latency_tier("tool.agent_utils", "get_current_time"),
            LatencyTier::B
        );
        assert_eq!(
            classify_latency_tier("tool.websearch", "search_status"),
            LatencyTier::B
        );
        assert_eq!(
            classify_latency_tier("tool.gaze", "start_tracking"),
            LatencyTier::B
        );
        assert_eq!(
            classify_latency_tier("tool.gaze", "stop_tracking"),
            LatencyTier::B
        );
        assert_eq!(
            classify_latency_tier("tool.gaze", "get_tracker_status"),
            LatencyTier::B
        );

        // Tier C (default)
        assert_eq!(
            classify_latency_tier("tool.terminal", "execute_command"),
            LatencyTier::C
        );
        assert_eq!(
            classify_latency_tier("tool.terminal", "read_file"),
            LatencyTier::C
        );
        assert_eq!(
            classify_latency_tier("tool.websearch", "web_search"),
            LatencyTier::C
        );
        assert_eq!(
            classify_latency_tier("tool.unknown", "anything"),
            LatencyTier::C
        );
    }
}
