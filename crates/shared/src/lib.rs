use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

pub mod llm;

// Legacy re-exports removed (cloto_macros, inventory) — all plugins are now MCP servers.

/// SDK version constant for consistent version reporting across all plugins
/// M-14: Plugins should reference this instead of their own CARGO_PKG_VERSION
pub const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Unique identifier within the Cloto platform (agents, plugins, sessions, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ClotoId(Uuid);

impl std::fmt::Display for ClotoId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// L-02: Default generates a random UUID v4 (intentional design).
/// Each default ClotoId is unique, suitable for trace IDs and ephemeral identifiers.
/// For deterministic IDs, use `ClotoId::from_name()` instead.
impl Default for ClotoId {
    fn default() -> Self {
        Self::new()
    }
}

impl ClotoId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Generate an ID for tracing.
    #[must_use]
    pub fn new_trace_id() -> Self {
        Self(Uuid::new_v4())
    }

    #[must_use]
    pub fn from_name(name: &str) -> Self {
        let namespace = Uuid::NAMESPACE_DNS;
        Self(Uuid::new_v5(&namespace, name.as_bytes()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CapabilityType {
    /// Thinking and inference (ReasoningEngine).
    Reasoning,
    /// Memory and persistence (MemoryProvider).
    Memory,
    /// External communication and I/O (CommunicationAdapter).
    Communication,
    /// Execution of a specific task (Tool).
    Tool,
    /// Vision and image processing.
    Vision,
    /// Speech transcription (Speech-to-Text).
    Stt,
    /// Speech output (Text-to-Speech / speak).
    Speech,
    /// Physical and hardware actuation.
    HAL,
    /// Web server extension (serves API endpoints).
    Web,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Permission {
    VisionRead,
    InputControl,
    FileRead,
    FileWrite,
    NetworkAccess,
    ProcessExecution,
    MemoryRead,
    MemoryWrite,
    AdminAccess,
}

impl std::fmt::Display for Permission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

// M-13: Explicit serde tagging for consistent serialization
#[derive(Debug, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "type", content = "detail")]
pub enum ClotoError {
    #[error("Permission denied: {0}")]
    PermissionDenied(Permission),
    #[error("Plugin error: {id} - {message}")]
    PluginError { id: String, message: String },
    #[error("Network error: {0}")]
    NetworkError(String),
    #[error("Timeout occurred: {0}")]
    Timeout(String),
    #[error("Configuration error: {0}")]
    ConfigError(String),
    #[error("Plugin not found: {0}")]
    PluginNotFound(String),
    #[error("Agent not found: {0}")]
    AgentNotFound(String),
    #[error("Internal error: {0}")]
    Internal(String),
    #[error("Validation error: {0}")]
    ValidationError(String),
}

pub type ClotoResult<T> = std::result::Result<T, ClotoError>;

/// Abstract storage interface plugins use to persist data (Principle #4: Data Sovereignty / Principle #6: SAL).
#[async_trait]
pub trait PluginDataStore: Send + Sync {
    /// Store a value as JSON.
    async fn set_json(
        &self,
        plugin_id: &str,
        key: &str,
        value: serde_json::Value,
    ) -> anyhow::Result<()>;
    /// Read a value stored as JSON.
    async fn get_json(
        &self,
        plugin_id: &str,
        key: &str,
    ) -> anyhow::Result<Option<serde_json::Value>>;
    /// Read every value whose key starts with the given prefix.
    async fn get_all_json(
        &self,
        plugin_id: &str,
        key_prefix: &str,
    ) -> anyhow::Result<Vec<(String, serde_json::Value)>>;

    /// Atomically increment a counter and return the updated value (prevents TOCTOU).
    async fn increment_counter(&self, plugin_id: &str, key: &str) -> anyhow::Result<i64> {
        // Default implementation: get -> increment -> set (non-atomic, test fallback).
        // SqliteDataStore overrides this with an atomic UPSERT.
        tracing::warn!(plugin_id = %plugin_id, key = %key,
            "Using non-atomic default increment_counter; override with atomic implementation for production use");
        let current = match self.get_json(plugin_id, key).await? {
            Some(val) => serde_json::from_value::<i64>(val).unwrap_or(0),
            None => 0,
        };
        let new_val = current + 1;
        self.set_json(plugin_id, key, serde_json::to_value(new_val)?)
            .await?;
        Ok(new_val)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
}

/// Response from a capability HTTP request.
///
/// Carries only the status code and body — there is no `headers` field, so a
/// caller cannot read a `Location` header. Combined with the no-redirect
/// contract below, this means a 3xx response is terminal from the caller's
/// point of view: it sees `status` in the 3xx range and an empty/short `body`,
/// and cannot follow the redirect itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

/// Sandboxed outbound HTTP capability, injected only when `NetworkAccess` is granted.
///
/// # Security contract (implementations MUST uphold)
/// - The target host MUST pass an allow-list check, and every resolved IP MUST
///   be rejected if it falls in a restricted range (loopback, link-local,
///   private, cloud-metadata, etc.). The connection MUST be pinned to the
///   validated addresses so DNS cannot be re-resolved to an unvalidated IP at
///   connect time (DNS-rebinding / TOCTOU defense — bug-407).
/// - **Redirects are NOT followed** (bug-414). Transparent redirect-following
///   would let a whitelisted host `302` to an internal IP that is re-resolved
///   without the allow-list / restricted-IP guard, bypassing every check in one
///   hop. A 3xx is therefore returned to the caller as-is (see `HttpResponse`),
///   not followed. Callers that need to act on a redirect must re-issue the
///   request against the (separately validated) target themselves; they cannot
///   read the `Location` header from `HttpResponse`.
#[async_trait]
pub trait NetworkCapability: Send + Sync {
    async fn send_http_request(&self, request: HttpRequest) -> anyhow::Result<HttpResponse>;
}

/// Sandboxed file I/O capability.
/// Only injected when FileRead and/or FileWrite permissions are granted.
/// Implementations MUST restrict access to a designated sandbox directory.
#[async_trait::async_trait]
pub trait FileCapability: Send + Sync {
    /// Read file contents. Path must be within the allowed sandbox directory.
    async fn read(&self, path: &str) -> anyhow::Result<Vec<u8>>;
    /// Write file contents. Only available when FileWrite is granted.
    async fn write(&self, path: &str, data: &[u8]) -> anyhow::Result<()>;
    /// Returns true if write operations are permitted.
    fn can_write(&self) -> bool;
}

/// Process execution capability.
/// Only injected when ProcessExecution permission is granted.
#[async_trait::async_trait]
pub trait ProcessCapability: Send + Sync {
    /// Execute a command. Returns (stdout, stderr, exit_code).
    async fn execute(&self, cmd: &str, args: &[String]) -> anyhow::Result<(String, String, i32)>;
}

/// Wrapper around a concrete capability injected at runtime.
#[derive(Clone)]
pub enum PluginCapability {
    Network(Arc<dyn NetworkCapability>),
    File(Arc<dyn FileCapability>),
    Process(Arc<dyn ProcessCapability>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceType {
    Communication,
    Reasoning,
    Skill,
    Vision,
    Action,
    Memory,
    HAL,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginCategory {
    Agent,  // A conversational persona (#MIND)
    Tool,   // A function or instrument (#TOOL)
    Memory, // Memory (#MEMORY)
    System, // Kernel internals (#SYSTEM)
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub category: PluginCategory,
    pub service_type: ServiceType,
    pub tags: Vec<String>,
    pub is_active: bool,
    pub is_configured: bool,
    pub required_config_keys: Vec<String>,
    pub action_icon: Option<String>,
    pub action_target: Option<String>,
    pub icon_data: Option<String>,
    pub magic_seal: u32,
    pub sdk_version: String,
    pub required_permissions: Vec<Permission>,
    pub provided_capabilities: Vec<CapabilityType>,
    pub provided_tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MessageSource {
    User { id: String, name: String },
    Agent { id: String },
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClotoMessage {
    pub id: String,
    pub source: MessageSource,
    pub target_agent: Option<String>,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub metadata: HashMap<String, String>,
}

impl ClotoMessage {
    #[must_use]
    pub fn new(source: MessageSource, content: String) -> Self {
        Self {
            id: ClotoId::new().to_string(),
            source,
            target_agent: None,
            content,
            timestamp: Utc::now(),
            metadata: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HandAction {
    MouseMove { x: i32, y: i32 },
    MouseClick { button: String },
    KeyPress { key: String },
    Wait { ms: u32 },
    // New Actions for Vision-HAL Coordination
    CaptureScreen,
    ClickElement { label: String },
}

/// Helper trait for downcasting a plugin to its concrete capability traits.
pub trait PluginCast {
    fn as_any(&self) -> &dyn Any;
    fn as_tool(&self) -> Option<&dyn Tool> {
        None
    }
    fn as_reasoning(&self) -> Option<&dyn ReasoningEngine> {
        None
    }
    fn as_memory(&self) -> Option<&dyn MemoryProvider> {
        None
    }
}

/// Base marker trait implemented by every plugin.
#[async_trait]
pub trait Plugin: Any + Send + Sync + PluginCast {
    fn manifest(&self) -> PluginManifest;

    /// Subscribe to system events (the default does nothing).
    /// Returning event data re-dispatches it through the kernel.
    async fn on_event(&self, _event: &ClotoEvent) -> anyhow::Result<Option<ClotoEventData>> {
        Ok(None)
    }

    /// Hook run when an agent is initialized (e.g. to inject metadata).
    async fn on_agent_init(&self, _agent: &mut AgentMetadata) -> anyhow::Result<()> {
        Ok(())
    }

    /// Hook run when a permission is granted at runtime and a capability is injected.
    async fn on_capability_injected(&self, _capability: PluginCapability) -> anyhow::Result<()> {
        Ok(())
    }
}

#[async_trait]
pub trait Tool: Plugin {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<serde_json::Value>;
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({})
    }
}

// ── Agentic Loop Types ──

/// Result of a reasoning step: either final text or tool call requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ThinkResult {
    /// Final text response — the agent is done thinking.
    Final(String),
    /// The LLM wants to call tools before continuing.
    ToolCalls {
        assistant_content: Option<String>,
        calls: Vec<ToolCall>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Tool call ID from the LLM (e.g., "call_abc123").
    pub id: String,
    /// Tool name (must match a registered Tool::name()).
    pub name: String,
    /// JSON arguments to pass to the tool.
    pub arguments: serde_json::Value,
}

// ── Tool Rejection Envelope (Option C — see docs/TOOL_REJECTION_TEST_PLAN.md) ──

/// Discriminator for structured tool rejections. Distinct from `anyhow::Error`
/// runtime failures — a rejection is a kernel-enforced policy/logical denial
/// that the agent can reason about and the operator can potentially resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RejectionCode {
    /// Privileged (YOLO) mode is required but currently disabled.
    YoloRequired,
    /// Access control policy denies this agent from using the tool.
    /// Phase G target — enum reserved for forward compatibility.
    AccessDenied,
    /// MCP server is not signed at the required trust level.
    /// Phase G target — enum reserved for forward compatibility.
    SealUnsigned,
    /// Tool classified as dangerous and not approved for autonomous execution.
    /// Dormant — no enforcer implemented yet; enum reserved for forward compatibility.
    RiskUnapproved,
    /// Generated MCP server code failed safety validation.
    /// Phase G target — enum reserved for forward compatibility.
    CodeUnsafe,
    /// Inter-agent delegation would form a cycle. Hard rejection.
    DelegationCycle,
    /// Inter-agent delegation chain exceeded maximum depth. Hard rejection.
    DelegationDepth,
    /// Delegation target equals caller. Hard rejection.
    SelfDelegation,
    /// External MCP server returned a rejection-shaped response that the
    /// kernel could not map to a specific code (Phase E fallback).
    Unknown,
}

/// Structured tool rejection payload. The `reason` field is self-contained
/// English that does not depend on the LLM understanding the code name —
/// keywords like "privileged mode", "currently disabled" are designed to
/// guide even weak local models. See docs/TOOL_REJECTION_TEST_PLAN.md §2
/// for the canonical text per code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRejection {
    pub code: RejectionCode,
    /// Human- and LLM-readable explanation, self-contained (no code-name dependency).
    pub reason: String,
    /// Short hint for the operator (e.g., "Enable YOLO in Settings → Security").
    /// `None` for hard rejections that no operator action can resolve.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation_hint: Option<String>,
    /// If `false`, the agentic loop breaks after a single occurrence
    /// (no retry can succeed). If `true`, the consecutive-same-code rule
    /// applies (break after 2 in a row).
    pub retryable: bool,
    /// Code-specific additional data (e.g., `{"chain": [...]}` for delegation,
    /// `{"violations": [...]}` for code safety).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// Failure result of a tool execution. Distinguishes structured policy
/// rejections (which the agent must recognize and the operator may resolve)
/// from runtime/resource errors (which flow through the existing error path).
///
/// `impl From<anyhow::Error>` provides `?`-operator compatibility so existing
/// kernel code that returns `anyhow::Result<Value>` can be migrated
/// incrementally to `Result<Value, ToolFailure>`.
#[derive(Debug)]
pub enum ToolFailure {
    /// Structured policy or logical rejection.
    Rejection(ToolRejection),
    /// Any other runtime failure (network, missing args, resource unavailable).
    Error(anyhow::Error),
}

impl From<anyhow::Error> for ToolFailure {
    fn from(err: anyhow::Error) -> Self {
        Self::Error(err)
    }
}

impl std::fmt::Display for ToolFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejection(r) => write!(f, "rejected ({:?}): {}", r.code, r.reason),
            Self::Error(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for ToolFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Rejection(_) => None,
            Self::Error(e) => e.source(),
        }
    }
}

#[async_trait]
pub trait ReasoningEngine: Plugin {
    fn name(&self) -> &str;

    async fn think(
        &self,
        agent: &AgentMetadata,
        message: &ClotoMessage,
        context: Vec<ClotoMessage>,
    ) -> anyhow::Result<String>;

    /// Whether this engine supports tool use (agentic loop). Default: false.
    fn supports_tools(&self) -> bool {
        false
    }

    /// Think with tool support. Default delegates to think() as Final.
    async fn think_with_tools(
        &self,
        agent: &AgentMetadata,
        message: &ClotoMessage,
        context: Vec<ClotoMessage>,
        _tools: &[serde_json::Value],
        _tool_history: &[serde_json::Value],
    ) -> anyhow::Result<ThinkResult> {
        let content = self.think(agent, message, context).await?;
        Ok(ThinkResult::Final(content))
    }
}

#[async_trait]
pub trait MemoryProvider: Plugin {
    fn name(&self) -> &str;
    async fn store(&self, agent_id: String, message: ClotoMessage) -> anyhow::Result<()>;
    async fn recall(
        &self,
        agent_id: String,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<ClotoMessage>>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClotoEvent {
    pub trace_id: ClotoId,
    pub timestamp: DateTime<Utc>,
    #[serde(flatten)]
    pub data: ClotoEventData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GazeData {
    pub x: i32,
    pub y: i32,
    pub confidence: f32,
    pub fixated: bool, // Whether the gaze has dwelled in place
}

/// Where an [`ClotoEventData::McpServerLog`] line originated. Shown as a badge
/// in the dashboard Log tab so operators can tell raw process output apart from
/// structured protocol logs. See `docs/MCP_SERVER_LOGS_DESIGN.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpLogSource {
    /// Raw line from the child process's stderr.
    Stderr,
    /// MCP `notifications/message` (the server's `logging` capability).
    McpLogging,
}

/// RFC 5424 / MCP log severity. Present for MCP-logging lines; absent for raw
/// stderr (which carries no reliable structure).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpLogLevel {
    Debug,
    Info,
    Notice,
    Warning,
    Error,
    Critical,
    Alert,
    Emergency,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
#[allow(clippy::large_enum_variant)]
pub enum ClotoEventData {
    MessageReceived(ClotoMessage),
    VisionUpdated {
        captured_at: DateTime<Utc>,
        detected_elements: Vec<serde_json::Value>,
        image_ref: Option<String>,
    },
    /// Gaze data was updated.
    GazeUpdated(GazeData),
    /// A plugin requested an action (subject to a permission check).
    ActionRequested {
        requester: ClotoId,
        action: HandAction,
    },
    SystemNotification(String),
    /// Ask a plugin to think (run inference).
    ThoughtRequested {
        agent: AgentMetadata,
        engine_id: String,
        message: ClotoMessage,
        context: Vec<ClotoMessage>,
    },
    /// A plugin returned its thinking result.
    ThoughtResponse {
        agent_id: String,
        engine_id: String,
        content: String,
        source_message_id: String,
        /// When true, the kernel has auto-spoken this response via a Speech-capable server.
        /// Dashboard should skip text-based viseme generation to avoid conflicts.
        #[serde(default)]
        auto_spoken: bool,
    },
    /// Start consensus-building across several plugins.
    ConsensusRequested {
        task: String,
        engine_ids: Vec<String>,
    },
    /// A plugin's configuration was updated.
    ConfigUpdated {
        plugin_id: String,
        config: std::collections::HashMap<String, String>,
    },
    /// P8: Runtime permission escalation request from a plugin/MCP server.
    /// Emitted when a server requests a capability it does not currently have.
    PermissionRequested {
        plugin_id: String,
        /// MGP permission string (e.g., "shell.execute", "network.outbound").
        permission: String,
        reason: String,
    },
    /// A permission was granted.
    PermissionGranted {
        plugin_id: String,
        /// MGP permission string (e.g., "shell.execute", "network.outbound").
        permission: String,
    },
    /// An agent's power state changed.
    AgentPowerChanged {
        agent_id: String,
        enabled: bool,
    },
    // ── Agentic Loop Events ──
    /// A tool was invoked during an agentic loop iteration (observability).
    ToolInvoked {
        agent_id: String,
        engine_id: String,
        tool_name: String,
        call_id: String,
        success: bool,
        duration_ms: u64,
        iteration: u8,
        /// Short summary of tool arguments for UI display (e.g., command name).
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_hint: Option<String>,
    },
    /// A tool invocation was structurally rejected by kernel policy or logic.
    /// Emitted in addition to `ToolInvoked { success: false }` for the same
    /// call. Dashboard renders this as a dedicated rejection card.
    ToolRejected {
        agent_id: String,
        engine_id: String,
        tool_name: String,
        call_id: String,
        code: RejectionCode,
        reason: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        remediation_hint: Option<String>,
        retryable: bool,
        iteration: u8,
        #[serde(skip_serializing_if = "Option::is_none")]
        details: Option<serde_json::Value>,
    },
    /// Intermediate reasoning text from the LLM before executing tool calls.
    AgentThinking {
        agent_id: String,
        engine_id: String,
        content: String,
        iteration: u8,
    },
    /// Per-token streaming delta from an MCP reasoning engine using MGP §12
    /// streaming (Phase C). Consumers concatenate deltas in `index` order to
    /// reconstruct the text generated so far. Only emitted when
    /// `CLOTO_MCP_STREAMING_ENABLED=true`. The final result still arrives as
    /// `ThoughtResponse` / `AgenticLoopCompleted` so consumers that ignore this
    /// variant stay functional.
    AgentTokenStream {
        agent_id: String,
        engine_id: String,
        delta: String,
        index: u32,
        iteration: u8,
        source_message_id: String,
    },
    /// An agentic loop completed (all tool calls resolved).
    AgenticLoopCompleted {
        agent_id: String,
        engine_id: String,
        total_iterations: u8,
        total_tool_calls: u32,
        source_message_id: String,
    },
    /// MCP server-initiated notification (Server→Kernel, MGP §13/§19.6)
    McpNotification {
        server_id: String,
        method: String,
        params: serde_json::Value,
    },
    /// MCP callback request from a server requiring operator/kernel response (MGP §13).
    McpCallbackRequested {
        callback_id: String,
        server_id: String,
        callback_type: String,
        message: String,
        options: Option<Vec<String>>,
        /// Bridge-specific metadata (e.g., channel_id, author_name for Discord).
        metadata: Option<serde_json::Value>,
    },
    /// A log line from an MCP/MGP server, surfaced to the dashboard Log tab.
    /// Unifies two sources (child stderr and the MCP `logging` capability),
    /// distinguished by `source`. See `docs/MCP_SERVER_LOGS_DESIGN.md`.
    McpServerLog {
        server_id: String,
        /// Where the line came from — rendered as a badge in the UI.
        source: McpLogSource,
        /// RFC 5424 severity. Present for MCP-logging lines; `None` for stderr.
        #[serde(skip_serializing_if = "Option::is_none")]
        level: Option<McpLogLevel>,
        /// Optional logger/category name (MCP logging `logger` field).
        #[serde(skip_serializing_if = "Option::is_none")]
        logger: Option<String>,
        message: String,
        /// RFC3339: kernel receive time (stderr) or notification time (MCP logging).
        timestamp: String,
    },
    /// Terminal commands require human approval before execution (batch).
    CommandApprovalRequested {
        approval_id: String,
        agent_id: String,
        /// List of commands pending approval: [{command, command_name}]
        commands: Vec<serde_json::Value>,
    },
    /// Result of a command approval decision.
    CommandApprovalResult {
        approval_id: String,
        /// "approved", "trusted", "denied", "timeout"
        decision: String,
    },
    /// External I/O action: an incoming message from an I/O bridge (Discord, Slack, etc.)
    /// processed through the agentic loop with automatic response routing via MGP callback.
    ExternalAction {
        action_id: String,
        /// Source bridge identifier (e.g., "discord").
        source: String,
        /// Human-readable label for the source (e.g., "Discord").
        source_label: String,
        target_agent_id: String,
        target_agent_name: String,
        /// The incoming message/prompt from the external user.
        prompt: String,
        /// Sender identity from the external platform.
        sender_name: String,
        engine_id: String,
        /// None while pending, Some when completed or failed.
        response: Option<String>,
        /// "pending", "success", "error"
        status: String,
        /// The callback_id used for response routing back to the I/O bridge.
        callback_id: String,
    },
    /// Inter-agent dialogue: one agent delegated a question to another via mgp.agent.ask.
    /// Emitted at request time (response = None) and at completion (response = Some).
    AgentDialogue {
        dialogue_id: String,
        caller_agent_id: String,
        caller_agent_name: String,
        target_agent_id: String,
        target_agent_name: String,
        prompt: String,
        engine_id: String,
        /// None while pending, Some when completed or failed.
        response: Option<String>,
        /// Delegation chain depth (1-based).
        chain_depth: u8,
        /// "pending", "success", "error"
        status: String,
    },
    /// Consensus deliberation progress: one entry per engine proposal and one
    /// for the synthesis, emitted by `SystemHandler::run_consensus` so the
    /// dashboard's dedicated "Consensus" actions tab can show the multi-engine
    /// deliberation live. Emitted at dispatch (`response = None`, status
    /// "pending") and at completion (status "success"/"error"). The final
    /// synthesized answer is ALSO delivered to the originating agent's chat as a
    /// normal `ThoughtResponse`; this event is the per-step visualization only.
    ConsensusProgress {
        /// Groups every step of one consensus session (= the request trace id).
        consensus_id: String,
        /// Originating agent the `consensus:` message was sent to.
        agent_id: String,
        agent_name: String,
        /// The user's question (without the consensus prefix is acceptable; the
        /// raw prompt is fine — the tab shows it once per session).
        prompt: String,
        /// "proposal" (one engine's contribution) or "synthesis" (the merge).
        phase: String,
        /// The engine producing this step (e.g. "deepseek").
        engine_id: String,
        /// Which sample of this engine produced the step. 1 = the first (primary)
        /// proposal; >= 2 = a reuse fallback re-sampling the same engine (through a
        /// varied framing) to reach quorum when too few distinct engines are
        /// available. Always 1 for synthesis.
        #[serde(default = "default_sample_index")]
        sample_index: u32,
        /// None while pending; Some(text) on success, Some(error message) on error.
        response: Option<String>,
        /// "pending", "success", "error".
        status: String,
        /// MGP §14 error code when `status = "error"` and the failing engine is
        /// an MGP server (e.g. 2000 SERVER_NOT_READY, 3003 TIMEOUT, 5000
        /// UPSTREAM_ERROR). None for non-MGP/internal failures.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mgp_error_code: Option<i64>,
        /// Whether the MGP error is retryable (from the error's recovery hints).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retryable: Option<bool>,
    },
}

impl ClotoEvent {
    #[must_use]
    pub fn new(data: ClotoEventData) -> Self {
        Self {
            trace_id: ClotoId::new_trace_id(),
            timestamp: Utc::now(),
            data,
        }
    }

    #[must_use]
    pub fn with_trace(trace_id: ClotoId, data: ClotoEventData) -> Self {
        Self {
            trace_id,
            timestamp: Utc::now(),
            data,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMetadata {
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub last_seen: i64,
    pub status: String,
    pub default_engine_id: Option<String>,
    pub required_capabilities: Vec<CapabilityType>,
    pub metadata: HashMap<String, String>,
    /// "agent" (user-created) or "system" (system.* agents). Defaults to "agent".
    #[serde(default = "default_agent_type")]
    pub agent_type: String,
}

fn default_agent_type() -> String {
    "agent".to_string()
}

fn default_sample_index() -> u32 {
    1
}

impl AgentMetadata {
    /// Resolve dynamic status from enabled flag and last_seen timestamp.
    pub fn resolve_status(&mut self, heartbeat_threshold_ms: i64) {
        self.status = if !self.enabled || self.last_seen == 0 {
            "offline".to_string()
        } else {
            let now_ms = chrono::Utc::now().timestamp_millis();
            let age_ms = now_ms - self.last_seen;
            if age_ms <= heartbeat_threshold_ms {
                "online".to_string()
            } else {
                "degraded".to_string()
            }
        };
    }
}

#[cfg(test)]
mod tool_rejection_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejection_code_serializes_as_screaming_snake_case() {
        let cases = [
            (RejectionCode::YoloRequired, "\"YOLO_REQUIRED\""),
            (RejectionCode::AccessDenied, "\"ACCESS_DENIED\""),
            (RejectionCode::SealUnsigned, "\"SEAL_UNSIGNED\""),
            (RejectionCode::RiskUnapproved, "\"RISK_UNAPPROVED\""),
            (RejectionCode::CodeUnsafe, "\"CODE_UNSAFE\""),
            (RejectionCode::DelegationCycle, "\"DELEGATION_CYCLE\""),
            (RejectionCode::DelegationDepth, "\"DELEGATION_DEPTH\""),
            (RejectionCode::SelfDelegation, "\"SELF_DELEGATION\""),
            (RejectionCode::Unknown, "\"UNKNOWN\""),
        ];
        for (code, expected) in cases {
            let actual = serde_json::to_string(&code).unwrap();
            assert_eq!(actual, expected, "code {:?}", code);
            let round: RejectionCode = serde_json::from_str(&actual).unwrap();
            assert_eq!(round, code);
        }
    }

    #[test]
    fn tool_rejection_roundtrip_minimal() {
        let rej = ToolRejection {
            code: RejectionCode::YoloRequired,
            reason: "This tool is restricted to privileged (YOLO) mode".to_string(),
            remediation_hint: Some("Enable YOLO in Settings".to_string()),
            retryable: true,
            details: None,
        };
        let s = serde_json::to_string(&rej).unwrap();
        let parsed: ToolRejection = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.code, RejectionCode::YoloRequired);
        assert!(parsed.reason.starts_with("This tool is restricted"));
        assert_eq!(
            parsed.remediation_hint.as_deref(),
            Some("Enable YOLO in Settings")
        );
        assert!(parsed.retryable);
        assert!(parsed.details.is_none());
        // Verify skip_serializing_if elides absent fields.
        assert!(!s.contains("\"details\""));
    }

    #[test]
    fn tool_rejection_roundtrip_with_details() {
        let rej = ToolRejection {
            code: RejectionCode::DelegationCycle,
            reason: "Inter-agent delegation would form a cycle".to_string(),
            remediation_hint: None,
            retryable: false,
            details: Some(json!({"chain": ["agent.a", "agent.b", "agent.a"]})),
        };
        let s = serde_json::to_string(&rej).unwrap();
        let parsed: ToolRejection = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.code, RejectionCode::DelegationCycle);
        assert!(!parsed.retryable);
        assert!(parsed.remediation_hint.is_none());
        assert_eq!(
            parsed
                .details
                .as_ref()
                .and_then(|d| d.get("chain"))
                .and_then(|c| c.as_array())
                .map(Vec::len),
            Some(3)
        );
        // Elided because None.
        assert!(!s.contains("\"remediation_hint\""));
    }

    #[test]
    fn tool_failure_from_anyhow_preserves_error() {
        let err = anyhow::anyhow!("missing required parameter: agent_id");
        let failure: ToolFailure = err.into();
        match failure {
            ToolFailure::Error(e) => {
                assert!(e.to_string().contains("missing required parameter"));
            }
            ToolFailure::Rejection(_) => panic!("expected Error variant"),
        }
    }

    #[test]
    fn tool_failure_question_mark_compat() {
        fn inner() -> Result<serde_json::Value, ToolFailure> {
            // Demonstrates that anyhow-returning code can be called with `?`
            // from a function returning `Result<_, ToolFailure>`.
            let v: serde_json::Value = serde_json::from_str("{\"ok\": true}")
                .map_err(|e| anyhow::anyhow!("parse failed: {}", e))?;
            Ok(v)
        }
        let result = inner().expect("should succeed");
        assert_eq!(result["ok"], json!(true));
    }

    #[test]
    fn tool_failure_display_formats_both_variants() {
        let rejection = ToolFailure::Rejection(ToolRejection {
            code: RejectionCode::YoloRequired,
            reason: "privileged mode disabled".to_string(),
            remediation_hint: None,
            retryable: true,
            details: None,
        });
        assert!(rejection.to_string().contains("YoloRequired"));
        assert!(rejection.to_string().contains("privileged mode disabled"));

        let error: ToolFailure = anyhow::anyhow!("runtime issue").into();
        assert_eq!(error.to_string(), "runtime issue");
    }

    #[test]
    fn tool_rejected_event_serializes_with_type_tag() {
        let event = ClotoEventData::ToolRejected {
            agent_id: "agent.test".to_string(),
            engine_id: "mind.deepseek".to_string(),
            tool_name: "mgp.access.grant".to_string(),
            call_id: "call_123".to_string(),
            code: RejectionCode::YoloRequired,
            reason: "privileged mode is disabled".to_string(),
            remediation_hint: Some("enable it in Settings".to_string()),
            retryable: true,
            iteration: 1,
            details: None,
        };
        let s = serde_json::to_string(&event).unwrap();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["type"], json!("ToolRejected"));
        assert_eq!(v["data"]["code"], json!("YOLO_REQUIRED"));
        assert_eq!(v["data"]["tool_name"], json!("mgp.access.grant"));
        assert_eq!(v["data"]["retryable"], json!(true));
        // Optional details field elided when None.
        assert!(v["data"].get("details").is_none());
    }

    #[test]
    fn tool_rejected_event_roundtrip() {
        let event = ClotoEventData::ToolRejected {
            agent_id: "agent.x".to_string(),
            engine_id: "mind.cerebras".to_string(),
            tool_name: "mgp.agent.ask".to_string(),
            call_id: "call_abc".to_string(),
            code: RejectionCode::DelegationCycle,
            reason: "cycle detected".to_string(),
            remediation_hint: None,
            retryable: false,
            iteration: 2,
            details: Some(json!({"chain": ["a", "b", "a"]})),
        };
        let s = serde_json::to_string(&event).unwrap();
        let parsed: ClotoEventData = serde_json::from_str(&s).unwrap();
        match parsed {
            ClotoEventData::ToolRejected {
                code,
                retryable,
                iteration,
                details,
                ..
            } => {
                assert_eq!(code, RejectionCode::DelegationCycle);
                assert!(!retryable);
                assert_eq!(iteration, 2);
                assert!(details.is_some());
            }
            _ => panic!("expected ToolRejected variant"),
        }
    }

    #[test]
    fn mcp_server_log_stderr_wire_contract() {
        // The dashboard Log tab reads fields from event.data and switches on
        // event.type (adjacent tag). A stderr line has no level/logger, which
        // must be elided. See docs/MCP_SERVER_LOGS_DESIGN.md §2/§5.
        let event = ClotoEventData::McpServerLog {
            server_id: "mind.local".to_string(),
            source: McpLogSource::Stderr,
            level: None,
            logger: None,
            message: "listening on stdio".to_string(),
            timestamp: "2026-07-01T07:12:00+00:00".to_string(),
        };
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&event).unwrap()).unwrap();
        assert_eq!(v["type"], json!("McpServerLog"));
        assert_eq!(v["data"]["server_id"], json!("mind.local"));
        assert_eq!(v["data"]["source"], json!("stderr"));
        assert_eq!(v["data"]["message"], json!("listening on stdio"));
        // Absent for stderr (skip_serializing_if None) — the UI shows no level badge.
        assert!(v["data"].get("level").is_none());
        assert!(v["data"].get("logger").is_none());
    }

    #[test]
    fn mcp_server_log_mcplogging_levels_serialize_lowercase() {
        // MCP-logging lines carry an RFC 5424 level + optional logger, both
        // rendered as badges. Levels serialize lowercase to match the spec.
        let event = ClotoEventData::McpServerLog {
            server_id: "cpersona".to_string(),
            source: McpLogSource::McpLogging,
            level: Some(McpLogLevel::Warning),
            logger: Some("db".to_string()),
            message: "slow query".to_string(),
            timestamp: "2026-07-01T07:12:00+00:00".to_string(),
        };
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&event).unwrap()).unwrap();
        assert_eq!(v["data"]["source"], json!("mcp_logging"));
        assert_eq!(v["data"]["level"], json!("warning"));
        assert_eq!(v["data"]["logger"], json!("db"));

        // Round-trip back to the typed variant.
        let parsed: ClotoEventData = serde_json::from_value(v).unwrap();
        match parsed {
            ClotoEventData::McpServerLog { source, level, .. } => {
                assert_eq!(source, McpLogSource::McpLogging);
                assert_eq!(level, Some(McpLogLevel::Warning));
            }
            _ => panic!("expected McpServerLog variant"),
        }
    }
}
