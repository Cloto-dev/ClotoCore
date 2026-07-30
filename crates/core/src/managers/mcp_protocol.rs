use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, RwLock};

// ============================================================
// JSON-RPC 2.0 Types
// ============================================================

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// A JSON-RPC error object a server sent us, carried through `anyhow` as a
/// typed error so callers can recover `code` / `data` instead of re-parsing a
/// formatted string. Era negotiation needs both: the code identifies
/// `UNSUPPORTED_PROTOCOL_VERSION` and `data.supported` lists the versions the
/// server can actually speak.
///
/// `Display` is byte-identical to the `anyhow!("RPC Error {code}: {message}")`
/// text this type replaced, so every log line, test assertion and
/// `qa/issue-registry.json` pattern written against it keeps matching.
#[derive(Debug, Clone)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RPC Error {}: {}", self.code, self.message)
    }
}

impl std::error::Error for RpcError {}

impl RpcError {
    /// Protocol versions listed in `error.data.supported` — the payload MCP
    /// defines for [`UNSUPPORTED_PROTOCOL_VERSION`]. Empty when absent or
    /// malformed (never an error: a server that omits it simply gives us
    /// nothing to negotiate down to).
    #[must_use]
    pub fn supported_versions(&self) -> Vec<String> {
        self.data
            .as_ref()
            .and_then(|d| d.get("supported"))
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Server→Client notification (JSON-RPC 2.0 notification: no `id`, has `method`)
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcNotification {
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

/// Unified Server→Client message parser.
/// Tries Notification first (requires `method` field), then Response (all-Optional fields).
/// Order matters: `#[serde(untagged)]` tries variants in order, and Response's all-Optional
/// fields would greedily match notification JSON if tried first (silently swallowing notifications).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcMessage {
    Notification(JsonRpcNotification),
    Response(JsonRpcResponse),
}

impl JsonRpcRequest {
    #[must_use]
    pub fn new(id: i64, method: &str, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: Some(Value::Number(id.into())),
            method: method.to_string(),
            params,
        }
    }

    #[must_use]
    pub fn notification(method: &str, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: None,
            method: method.to_string(),
            params,
        }
    }
}

// ============================================================
// MCP Standard Types
// ============================================================

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub protocol_version: String,
    pub capabilities: ClientCapabilities,
    pub client_info: ClientInfo,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mgp: Option<super::mcp_mgp::MgpClientCapabilities>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct McpTool {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Value,
    /// MCP tool annotations (destructiveHint, readOnlyHint, etc.)
    #[serde(default)]
    pub annotations: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListToolsResult {
    pub tools: Vec<McpTool>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallToolParams {
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CallToolResult {
    pub content: Vec<ToolContent>,
    pub is_error: Option<bool>,
    /// MCP 2026-07-28 result discriminator (`"complete"` | `"input_required"`).
    /// Absent on every handshake-era response, so `default` keeps legacy
    /// parsing untouched. See [`RESULT_TYPE_INPUT_REQUIRED`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_type: Option<String>,
}

// ============================================================
// MCP 2026-07-28 "stateless core" (modern era)
// ============================================================

/// Modern ("stateless core") protocol versions this kernel can speak, oldest
/// first. MCP 2026-07-28 removed the `initialize` / `initialized` handshake:
/// every request carries its own context in `params._meta` instead.
pub const MODERN_PROTOCOL_VERSIONS: &[&str] = &["2026-07-28"];

/// Newest entry of [`MODERN_PROTOCOL_VERSIONS`] — the version the kernel probes
/// with first.
pub const MODERN_PROTOCOL_VERSION: &str = "2026-07-28";

/// Versions negotiated through the `initialize` handshake. Used to tell a
/// downgradeable server ("also speaks a handshake era") apart from a genuinely
/// incompatible modern-only one.
pub const HANDSHAKE_PROTOCOL_VERSIONS: &[&str] =
    &["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];

/// `protocolVersion` the kernel sends in the legacy `initialize` request.
/// Deliberately unchanged from the pre-dual-era client — the legacy path must
/// stay byte-identical on the wire.
pub const LEGACY_PROTOCOL_VERSION: &str = "2024-11-05";

/// Discovery method that replaces `initialize` in the modern era.
pub const DISCOVER_METHOD: &str = "server/discover";

/// Upper bound for a single `server/discover` probe, matching the reference
/// SDK. The effective probe timeout is `min(request_timeout, this)`.
pub const DISCOVER_PROBE_TIMEOUT_SECS: u64 = 10;

/// JSON-RPC error code for `UNSUPPORTED_PROTOCOL_VERSION`; `error.data.supported`
/// carries the versions the server does speak.
pub const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

// Per-request `_meta` keys (reference SDK `mcp` 2.0.0).
pub const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
pub const META_CLIENT_INFO: &str = "io.modelcontextprotocol/clientInfo";
pub const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
pub const META_LOG_LEVEL: &str = "io.modelcontextprotocol/logLevel";
/// Display-only server identity in `DiscoverResult._meta`. Absence or a
/// malformed value is never fatal.
pub const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

/// MGP extension key under `capabilities.extensions` (server side) and
/// `clientCapabilities.extensions` (kernel side) — mgp-spec 0.8.0-draft.
pub const MGP_CAPABILITY_EXTENSION: &str = "dev.cloto/mgp";

/// `_meta` key carrying approved MGP permission grants on `tools/call`
/// (mgp-spec 0.8.0-draft; modern era only — legacy uses the grant RPC).
pub const META_MGP_GRANTS: &str = "dev.cloto/mgp/grants";

/// Streamable-HTTP headers the modern era adds so a server's era router can
/// dispatch without parsing the body.
pub const HEADER_PROTOCOL_VERSION: &str = "mcp-protocol-version";
pub const HEADER_METHOD: &str = "mcp-method";

/// `resultType` values (MCP 2026-07-28). `input_required` is the multi-round
/// tool interaction (MRTR) continuation the kernel host has no flow for.
pub const RESULT_TYPE_COMPLETE: &str = "complete";
pub const RESULT_TYPE_INPUT_REQUIRED: &str = "input_required";

/// Which MCP era a connection speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProtocolEra {
    /// `initialize` / `initialized` handshake, session-scoped state.
    Legacy,
    /// MCP 2026-07-28 stateless core: no handshake, per-request `_meta`.
    Modern,
}

impl ProtocolEra {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::Modern => "modern",
        }
    }
}

/// Per-server override for era detection (`McpServerConfig.protocol_era`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EraPreference {
    /// Probe `server/discover`, fall back to `initialize` (default).
    #[default]
    Auto,
    /// Escape hatch: skip the probe and go straight to `initialize`, exactly as
    /// the pre-dual-era client did.
    LegacyOnly,
}

impl EraPreference {
    /// Parse the config value. `None` / `"auto"` probes; `"legacy"` skips the
    /// probe. Anything else warns and behaves as `Auto` — a typo must not
    /// silently pin a server to one era.
    #[must_use]
    pub fn from_config(raw: Option<&str>) -> Self {
        match raw.map(str::trim) {
            None | Some("") => Self::Auto,
            Some(v) if v.eq_ignore_ascii_case("auto") => Self::Auto,
            Some(v) if v.eq_ignore_ascii_case("legacy") => Self::LegacyOnly,
            Some(other) => {
                tracing::warn!(
                    value = %other,
                    "Unknown protocol_era in MCP server config — expected \"auto\" or \"legacy\"; using auto"
                );
                Self::Auto
            }
        }
    }
}

/// Result of `server/discover` (modern era). Only `supportedVersions` is
/// required: it is the field the era decision rests on, so a reply without it
/// is not a usable modern reply and must fall back to the handshake rather than
/// be treated as a modern server with unknown capabilities.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverResult {
    pub supported_versions: Vec<String>,
    #[serde(default)]
    pub ttl_ms: Option<i64>,
    #[serde(default)]
    pub cache_scope: Option<String>,
    #[serde(default)]
    pub capabilities: Option<Value>,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub result_type: Option<String>,
    #[serde(default, rename = "_meta")]
    pub meta: Option<Value>,
}

impl DiscoverResult {
    /// Highest modern version both sides speak, or `None` when the server
    /// advertises no modern version at all (some SDKs answer `server/discover`
    /// while only speaking handshake versions → legacy fallback).
    #[must_use]
    pub fn mutual_modern_version(&self) -> Option<&'static str> {
        highest_mutual_modern_version(&self.supported_versions)
    }

    /// `_meta` serverInfo rendered for logs (`"name vX"`). Display-only, so any
    /// unexpected shape degrades to `None` instead of failing the connection.
    #[must_use]
    pub fn server_info_display(&self) -> Option<String> {
        let info = self.meta.as_ref()?.get(META_SERVER_INFO)?;
        let name = info.get("name").and_then(Value::as_str)?;
        Some(match info.get("version").and_then(Value::as_str) {
            Some(v) => format!("{name} v{v}"),
            None => name.to_string(),
        })
    }
}

/// Highest version present both in [`MODERN_PROTOCOL_VERSIONS`] and in
/// `offered`. Dates sort chronologically as strings, so the last match in the
/// (ascending) constant is the newest.
#[must_use]
pub fn highest_mutual_modern_version(offered: &[String]) -> Option<&'static str> {
    MODERN_PROTOCOL_VERSIONS
        .iter()
        .rev()
        .find(|v| offered.iter().any(|o| o == *v))
        .copied()
}

/// True when `offered` contains at least one handshake-era version — i.e. the
/// server can still be reached through `initialize`. A server that offers
/// neither a mutual modern version nor any handshake version is genuinely
/// incompatible.
#[must_use]
pub fn offers_handshake_version(offered: &[String]) -> bool {
    offered
        .iter()
        .any(|o| HANDSHAKE_PROTOCOL_VERSIONS.contains(&o.as_str()))
}

/// Shared, cheap-to-read view of the negotiated era.
///
/// Written once by `McpClient::negotiate`, read by the client on every request
/// (`_meta` stamping, `resultType` gating) and by `HttpTransport`'s request
/// loop (era headers, `Mcp-Session-Id` suppression). The transport is started
/// *before* negotiation runs, so it needs a handle to state it does not own.
#[derive(Debug, Clone, Default)]
pub struct EraHandle {
    era: Arc<AtomicU8>,
    /// Version to advertise on the wire: the in-flight probe version before the
    /// era is settled, the negotiated version afterwards.
    version: Arc<RwLock<Option<String>>>,
}

impl EraHandle {
    const UNSET: u8 = 0;
    const LEGACY: u8 = 1;
    const MODERN: u8 = 2;

    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Negotiated era, or `None` while negotiation is still in flight.
    #[must_use]
    pub fn era(&self) -> Option<ProtocolEra> {
        match self.era.load(Ordering::SeqCst) {
            Self::LEGACY => Some(ProtocolEra::Legacy),
            Self::MODERN => Some(ProtocolEra::Modern),
            Self::UNSET => None,
            other => {
                debug_assert!(false, "unknown EraHandle discriminant {other}");
                None
            }
        }
    }

    /// True only once the modern era is settled — never during negotiation, so
    /// a probe cannot be mistaken for a negotiated modern connection.
    #[must_use]
    pub fn is_modern(&self) -> bool {
        self.era.load(Ordering::SeqCst) == Self::MODERN
    }

    pub fn set_legacy(&self) {
        self.era.store(Self::LEGACY, Ordering::SeqCst);
    }

    pub fn set_modern(&self, version: &str) {
        self.set_wire_version(version);
        self.era.store(Self::MODERN, Ordering::SeqCst);
    }

    /// Record the version an in-flight `server/discover` probe is using, so the
    /// HTTP transport can put the matching `mcp-protocol-version` header on it
    /// before any era is settled.
    pub fn set_wire_version(&self, version: &str) {
        if let Ok(mut guard) = self.version.write() {
            *guard = Some(version.to_string());
        }
    }

    /// Version to advertise on the wire, if known.
    #[must_use]
    pub fn wire_version(&self) -> Option<String> {
        self.version.read().ok().and_then(|g| g.clone())
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ToolContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { data: String, mime_type: String },
    #[serde(rename = "resource")]
    Resource { resource: Value },
}

// ============================================================
// Streaming Types (MGP §12)
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChunk {
    pub request_id: i64,
    pub index: u32,
    pub content: ToolContent,
    pub done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamProgress {
    pub request_id: i64,
    pub progress: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_remaining_ms: Option<u64>,
}

// ============================================================
// Cloto Custom MCP Extensions
// ============================================================

/// Request params for cloto/handshake custom method
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClotoHandshakeParams {
    pub kernel_version: String,
}

/// Response from cloto/handshake
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ClotoHandshakeResult {
    pub server_id: String,
    pub version: Option<String>,
    pub capabilities: Vec<String>,
    pub tools: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seal: Option<String>,
}

// ============================================================
// Restart Policy (MGP §11)
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestartStrategy {
    Never,
    OnFailure,
    Always,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestartPolicy {
    pub strategy: RestartStrategy,
    #[serde(default = "default_max_restarts")]
    pub max_restarts: u32,
    #[serde(default = "default_restart_window_secs")]
    pub restart_window_secs: u64,
    #[serde(default = "default_backoff_base_ms")]
    pub backoff_base_ms: u64,
    #[serde(default = "default_backoff_max_ms")]
    pub backoff_max_ms: u64,
}

/// Default restart-policy values (bug-313). Single source of truth shared by the
/// serde `#[serde(default = ...)]` helpers, `RestartPolicy::default()`, and tests,
/// so the runtime defaults can no longer drift from each other or from the docs.
/// Documented in `docs/ARCHITECTURE.md` (MCP Server Restart Policy).
pub const DEFAULT_MAX_RESTARTS: u32 = 5;
pub const DEFAULT_RESTART_WINDOW_SECS: u64 = 300;
pub const DEFAULT_BACKOFF_BASE_MS: u64 = 1000;
pub const DEFAULT_BACKOFF_MAX_MS: u64 = 30000;

fn default_max_restarts() -> u32 {
    DEFAULT_MAX_RESTARTS
}
fn default_restart_window_secs() -> u64 {
    DEFAULT_RESTART_WINDOW_SECS
}
fn default_backoff_base_ms() -> u64 {
    DEFAULT_BACKOFF_BASE_MS
}
fn default_backoff_max_ms() -> u64 {
    DEFAULT_BACKOFF_MAX_MS
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            strategy: RestartStrategy::OnFailure,
            max_restarts: default_max_restarts(),
            restart_window_secs: default_restart_window_secs(),
            backoff_base_ms: default_backoff_base_ms(),
            backoff_max_ms: default_backoff_max_ms(),
        }
    }
}

/// MCP Server configuration (from mcp.toml or database)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub id: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    #[serde(default = "default_transport")]
    pub transport: String,
    /// URL for HTTP-based transports (required when transport = "streamable-http").
    #[serde(default)]
    pub url: Option<String>,
    /// Authentication token for HTTP transport (Bearer token).
    #[serde(default)]
    pub auth_token: Option<String>,
    /// Legacy field — prefer `restart_policy`. When restart_policy is None,
    /// auto_restart controls fallback: Some(true) → OnFailure, Some(false)/None → Never.
    #[serde(default)]
    pub auto_restart: Option<bool>,
    /// Required permissions for this MCP server (Permission gate: D).
    /// In non-YOLO mode, all permissions must be approved before the server starts.
    #[serde(default)]
    pub required_permissions: Vec<String>,
    /// Human-readable display name for the UI (e.g., "DeepSeek", "Cerebras").
    #[serde(default)]
    pub display_name: Option<String>,
    /// MGP configuration for this server (optional, from mcp.toml `[servers.mgp]`).
    #[serde(default)]
    pub mgp: Option<super::mcp_mgp::MgpServerConfig>,
    /// Restart policy for this server (MGP §11).
    #[serde(default)]
    pub restart_policy: Option<RestartPolicy>,
    /// HMAC-SHA256 seal of the server entry point (MGP §8 L0: Magic Seal).
    #[serde(default)]
    pub seal: Option<String>,
    /// Per-server isolation config overrides (MGP §8-10).
    #[serde(default)]
    pub isolation: Option<super::mcp_isolation::IsolationConfig>,
    /// ClotoHub catalog connector id when this server was installed via
    /// `/api/marketplace/install`. NULL ⇒ manually registered (CLI / API / mcp.toml).
    /// Surfaced in `McpServerInfo.marketplace_id` so the dashboard can render
    /// the MGP purple card for catalog-originated servers even when unsigned
    /// (per MGP §10 inv 3: seal absence demotes trust_level, not MGP membership).
    #[serde(default)]
    pub marketplace_id: Option<String>,
    /// Era-detection override: `None` / `"auto"` probes `server/discover` and
    /// falls back to `initialize`; `"legacy"` skips the probe entirely (escape
    /// hatch for a server that mishandles unknown methods). Parsed by
    /// [`EraPreference::from_config`] — an unknown value warns and acts as auto.
    #[serde(default)]
    pub protocol_era: Option<String>,
}

fn default_transport() -> String {
    "stdio".to_string()
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            command: String::new(),
            args: Vec::new(),
            env: std::collections::HashMap::new(),
            transport: default_transport(),
            url: None,
            auth_token: None,
            auto_restart: None,
            required_permissions: Vec::new(),
            display_name: None,
            mgp: None,
            restart_policy: None,
            seal: None,
            isolation: None,
            marketplace_id: None,
            protocol_era: None,
        }
    }
}

impl McpServerConfig {
    /// Returns the effective restart policy, respecting legacy auto_restart fallback.
    #[must_use]
    pub fn effective_restart_policy(&self) -> RestartPolicy {
        self.restart_policy.clone().unwrap_or_else(|| {
            if self.auto_restart.unwrap_or(false) {
                RestartPolicy::default() // OnFailure
            } else {
                RestartPolicy {
                    strategy: RestartStrategy::Never,
                    ..Default::default()
                }
            }
        })
    }
}

/// Top-level config structure for mcp.toml
#[derive(Debug, Deserialize)]
pub struct McpConfigFile {
    /// Path variables for resolving `${var}` in server args/command.
    /// Example: `[paths] servers = "C:/path/to/clotohub-servers/servers"`
    #[serde(default)]
    pub paths: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The probe version must be the newest modern version, or a server that
    /// speaks both an old and a new modern version would be pinned to the old
    /// one.
    #[test]
    fn modern_protocol_version_is_the_newest_known() {
        assert_eq!(
            Some(&MODERN_PROTOCOL_VERSION),
            MODERN_PROTOCOL_VERSIONS.last()
        );
        // Dates sort chronologically as strings — the ordering the version
        // selection relies on.
        let mut sorted = MODERN_PROTOCOL_VERSIONS.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, MODERN_PROTOCOL_VERSIONS.to_vec());
    }

    /// The legacy handshake version must stay exactly what the pre-dual-era
    /// client sent — the legacy path is meant to be byte-identical.
    #[test]
    fn legacy_protocol_version_is_unchanged() {
        assert_eq!(LEGACY_PROTOCOL_VERSION, "2024-11-05");
        assert!(HANDSHAKE_PROTOCOL_VERSIONS.contains(&LEGACY_PROTOCOL_VERSION));
        // The eras must not overlap, or era detection would be ambiguous.
        for modern in MODERN_PROTOCOL_VERSIONS {
            assert!(
                !HANDSHAKE_PROTOCOL_VERSIONS.contains(modern),
                "{modern} is listed as both a modern and a handshake version"
            );
        }
    }

    #[test]
    fn rpc_error_display_matches_the_legacy_format() {
        // Byte-identical to the anyhow!("RPC Error {}: {}") string this type
        // replaced — logs, tests and issue-registry patterns depend on it.
        let err = RpcError {
            code: -32601,
            message: "Method not found".to_string(),
            data: None,
        };
        assert_eq!(err.to_string(), "RPC Error -32601: Method not found");
    }

    #[test]
    fn rpc_error_supported_versions_reads_the_minus_32022_payload() {
        let err = RpcError {
            code: UNSUPPORTED_PROTOCOL_VERSION,
            message: "unsupported".to_string(),
            data: Some(serde_json::json!({ "supported": ["2026-07-28", "2025-06-18"] })),
        };
        assert_eq!(err.supported_versions(), vec!["2026-07-28", "2025-06-18"]);

        // Missing / malformed payloads degrade to empty, never to an error.
        let no_data = RpcError {
            code: UNSUPPORTED_PROTOCOL_VERSION,
            message: String::new(),
            data: None,
        };
        assert!(no_data.supported_versions().is_empty());
        let wrong_shape = RpcError {
            code: UNSUPPORTED_PROTOCOL_VERSION,
            message: String::new(),
            data: Some(serde_json::json!({ "supported": "2026-07-28" })),
        };
        assert!(wrong_shape.supported_versions().is_empty());
        let mixed = RpcError {
            code: UNSUPPORTED_PROTOCOL_VERSION,
            message: String::new(),
            data: Some(serde_json::json!({ "supported": ["2026-07-28", 7, null] })),
        };
        assert_eq!(mixed.supported_versions(), vec!["2026-07-28"]);
    }

    /// The escape hatch documented in `docs/ARCHITECTURE.md` §3.1.3 is reached
    /// through `mcp.toml`, so the whole path — TOML field → config → era
    /// preference — is pinned here rather than only the parser half. A server
    /// entry that omits the key must keep probing.
    #[test]
    fn protocol_era_escape_hatch_survives_mcp_toml() {
        let parsed: McpConfigFile = toml::from_str(
            r#"
[[servers]]
id = "pinned"
command = "python"
protocol_era = "legacy"

[[servers]]
id = "probed"
command = "python"
"#,
        )
        .expect("mcp.toml with protocol_era must parse");

        assert_eq!(parsed.servers[0].protocol_era.as_deref(), Some("legacy"));
        assert_eq!(
            EraPreference::from_config(parsed.servers[0].protocol_era.as_deref()),
            EraPreference::LegacyOnly,
            "protocol_era=\"legacy\" in mcp.toml must skip the discover probe"
        );

        assert!(parsed.servers[1].protocol_era.is_none());
        assert_eq!(
            EraPreference::from_config(parsed.servers[1].protocol_era.as_deref()),
            EraPreference::Auto,
            "a server entry without protocol_era must still probe"
        );
    }

    #[test]
    fn era_preference_parses_config_values() {
        assert_eq!(EraPreference::from_config(None), EraPreference::Auto);
        assert_eq!(EraPreference::from_config(Some("")), EraPreference::Auto);
        assert_eq!(
            EraPreference::from_config(Some("auto")),
            EraPreference::Auto
        );
        assert_eq!(
            EraPreference::from_config(Some("AUTO")),
            EraPreference::Auto
        );
        assert_eq!(
            EraPreference::from_config(Some(" legacy ")),
            EraPreference::LegacyOnly
        );
        assert_eq!(
            EraPreference::from_config(Some("Legacy")),
            EraPreference::LegacyOnly
        );
        // An unknown value must not silently pin an era — it degrades to auto,
        // which still reaches a legacy server through the fallback.
        assert_eq!(
            EraPreference::from_config(Some("modern")),
            EraPreference::Auto
        );
        assert_eq!(
            EraPreference::from_config(Some("2026-07-28")),
            EraPreference::Auto
        );
    }

    #[test]
    fn mutual_modern_version_prefers_the_newest_and_handles_disjoint() {
        assert_eq!(
            highest_mutual_modern_version(&["2026-07-28".to_string()]),
            Some("2026-07-28")
        );
        // A future modern version we do not know is not mutual.
        assert_eq!(
            highest_mutual_modern_version(&["2027-01-01".to_string()]),
            None
        );
        assert_eq!(highest_mutual_modern_version(&[]), None);
        assert_eq!(
            highest_mutual_modern_version(&["2025-06-18".to_string()]),
            None
        );
    }

    #[test]
    fn offers_handshake_version_detects_a_downgrade_path() {
        assert!(offers_handshake_version(&["2025-06-18".to_string()]));
        assert!(offers_handshake_version(&[
            "2027-01-01".to_string(),
            "2024-11-05".to_string()
        ]));
        // Modern-only (and unknown-modern-only) servers have no handshake path:
        // this is the "genuinely incompatible" signal.
        assert!(!offers_handshake_version(&["2027-01-01".to_string()]));
        assert!(!offers_handshake_version(&[]));
    }

    #[test]
    fn discover_result_parses_the_reference_wire_shape() {
        let raw = serde_json::json!({
            "ttlMs": 60000,
            "cacheScope": "private",
            "supportedVersions": ["2026-07-28"],
            "capabilities": { "extensions": { "dev.cloto/mgp": { "version": "0.6.0" } } },
            "instructions": "call echo first",
            "resultType": "complete",
            "_meta": {
                "io.modelcontextprotocol/serverInfo": { "name": "demo", "version": "1.2.3" }
            }
        });
        let parsed: DiscoverResult = serde_json::from_value(raw).expect("reference shape parses");
        assert_eq!(parsed.ttl_ms, Some(60000));
        assert_eq!(parsed.cache_scope.as_deref(), Some("private"));
        assert_eq!(parsed.mutual_modern_version(), Some("2026-07-28"));
        assert_eq!(parsed.instructions.as_deref(), Some("call echo first"));
        assert_eq!(parsed.result_type.as_deref(), Some(RESULT_TYPE_COMPLETE));
        assert_eq!(parsed.server_info_display().as_deref(), Some("demo v1.2.3"));
    }

    #[test]
    fn discover_result_tolerates_everything_but_supported_versions() {
        // Minimal reply: only the era-deciding field.
        let minimal: DiscoverResult =
            serde_json::from_value(serde_json::json!({ "supportedVersions": ["2026-07-28"] }))
                .expect("supportedVersions alone is a usable modern reply");
        assert!(minimal.capabilities.is_none());
        assert!(minimal.server_info_display().is_none());

        // serverInfo is display-only: a malformed one must not fail parsing.
        let bad_info: DiscoverResult = serde_json::from_value(serde_json::json!({
            "supportedVersions": ["2026-07-28"],
            "_meta": { "io.modelcontextprotocol/serverInfo": 42 }
        }))
        .expect("a malformed serverInfo must not break the connection");
        assert!(bad_info.server_info_display().is_none());

        // No supportedVersions → not a usable modern reply → the client falls
        // back to the handshake rather than guessing.
        assert!(
            serde_json::from_value::<DiscoverResult>(serde_json::json!({ "ttlMs": 1 })).is_err()
        );
    }

    #[test]
    fn era_handle_starts_unset_and_settles_once() {
        let handle = EraHandle::new();
        assert_eq!(handle.era(), None);
        assert!(!handle.is_modern());
        assert_eq!(handle.wire_version(), None);

        // A probe publishes its version before any era is settled, so the HTTP
        // transport can label the probe request itself.
        handle.set_wire_version("2026-07-28");
        assert!(!handle.is_modern(), "a probe is not a settled modern era");
        assert_eq!(handle.wire_version().as_deref(), Some("2026-07-28"));

        handle.set_modern("2026-07-28");
        assert_eq!(handle.era(), Some(ProtocolEra::Modern));
        assert!(handle.is_modern());

        let legacy = EraHandle::new();
        legacy.set_legacy();
        assert_eq!(legacy.era(), Some(ProtocolEra::Legacy));
        assert!(!legacy.is_modern());
    }

    #[test]
    fn era_handle_clones_share_state() {
        // The HTTP transport holds a clone made before negotiation runs; it must
        // observe the era the client settles on.
        let handle = EraHandle::new();
        let transport_view = handle.clone();
        handle.set_modern(MODERN_PROTOCOL_VERSION);
        assert!(transport_view.is_modern());
        assert_eq!(
            transport_view.wire_version().as_deref(),
            Some(MODERN_PROTOCOL_VERSION)
        );
    }

    #[test]
    fn call_tool_result_result_type_is_additive() {
        // Legacy responses have no resultType and must still parse.
        let legacy: CallToolResult = serde_json::from_value(serde_json::json!({
            "content": [{ "type": "text", "text": "hi" }],
            "isError": false
        }))
        .expect("a handshake-era tool result must still parse");
        assert!(legacy.result_type.is_none());

        let modern: CallToolResult = serde_json::from_value(serde_json::json!({
            "content": [{ "type": "text", "text": "hi" }],
            "isError": false,
            "resultType": "complete"
        }))
        .expect("a modern tool result parses");
        assert_eq!(modern.result_type.as_deref(), Some(RESULT_TYPE_COMPLETE));
    }

    #[test]
    fn protocol_era_config_field_defaults_to_none() {
        // Additive: every existing mcp.toml / DB-derived config keeps working.
        let cfg: McpServerConfig = serde_json::from_value(serde_json::json!({
            "id": "demo", "command": "python3"
        }))
        .expect("config without protocol_era parses");
        assert!(cfg.protocol_era.is_none());
        assert_eq!(
            EraPreference::from_config(cfg.protocol_era.as_deref()),
            EraPreference::Auto
        );
    }
}
