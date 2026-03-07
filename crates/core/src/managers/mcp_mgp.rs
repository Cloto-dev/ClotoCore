//! MGP (Model General Protocol) Tier 1 — Capability Negotiation + Tool Security Metadata.
//!
//! MGP is a strict superset of MCP. This module implements Tier 1:
//! - `mgp` field in `initialize` handshake (capability negotiation)
//! - `tool_security` metadata on `tools/list` responses
//! - `effective_risk_level` derivation (§4.6)

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Current MGP protocol version.
pub const MGP_VERSION: &str = "0.6.0";

/// Extensions offered by the kernel in Tier 1.
pub const TIER1_CLIENT_EXTENSIONS: &[&str] = &["tool_security"];

/// Extensions offered by the kernel (Tier 2 Security + Tier 3 Communication + Tier 4 Intelligence).
pub const CLIENT_EXTENSIONS: &[&str] = &[
    "tool_security",
    "permissions",
    "access_control",
    "audit",
    "code_safety",
    "error_handling",
    "lifecycle",
    "streaming",
    "progress",
    "events",
    "callbacks",
    "discovery",       // Tier 4: §15 Server Discovery
    "tool_discovery",  // Tier 4: §16 Dynamic Tool Discovery
];

// ============================================================
// MGP Error Codes (§14.3)
// ============================================================

// Security errors (1000–1099)
pub const MGP_ERR_PERMISSION_DENIED: i64 = 1000;
pub const MGP_ERR_ACCESS_DENIED: i64 = 1001;
pub const MGP_ERR_AUTH_REQUIRED: i64 = 1002;
pub const MGP_ERR_AUTH_EXPIRED: i64 = 1003;
pub const MGP_ERR_VALIDATION_BLOCKED: i64 = 1010;
pub const MGP_ERR_CODE_SAFETY_VIOLATION: i64 = 1011;

// Lifecycle errors (2000–2099)
pub const MGP_ERR_SERVER_NOT_READY: i64 = 2000;
pub const MGP_ERR_SERVER_DRAINING: i64 = 2001;
pub const MGP_ERR_SERVER_RESTARTING: i64 = 2002;

// Resource errors (3000–3099)
pub const MGP_ERR_RATE_LIMITED: i64 = 3000;
pub const MGP_ERR_RESOURCE_EXHAUSTED: i64 = 3001;
pub const MGP_ERR_QUOTA_EXCEEDED: i64 = 3002;
pub const MGP_ERR_TIMEOUT: i64 = 3003;

// Validation errors (4000–4099)
pub const MGP_ERR_INVALID_TOOL_ARGS: i64 = 4000;
pub const MGP_ERR_TOOL_NOT_FOUND: i64 = 4001;
pub const MGP_ERR_TOOL_DISABLED: i64 = 4002;
pub const MGP_ERR_TOOL_NAME_CONFLICT: i64 = 4003;

// Discovery errors (4100–4199)
#[allow(dead_code)]
pub const MGP_ERR_DISCOVERY_UNAVAILABLE: i64 = 4100;
#[allow(dead_code)]
pub const MGP_ERR_SERVER_ALREADY_REGISTERED: i64 = 4101;
#[allow(dead_code)]
pub const MGP_ERR_CANNOT_DEREGISTER_CONFIG: i64 = 4102;

// External service errors (5000–5099)
pub const MGP_ERR_UPSTREAM_ERROR: i64 = 5000;
pub const MGP_ERR_UPSTREAM_TIMEOUT: i64 = 5001;
pub const MGP_ERR_UPSTREAM_UNAVAILABLE: i64 = 5002;

// ============================================================
// MGP Error Recovery (§14.5)
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MgpErrorRecovery {
    pub category: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_strategy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

/// Build a JSON-RPC error `data` field with `_mgp` recovery hints (§14.4).
///
/// Recovery fields are placed directly under `_mgp` (not nested under `_mgp.recovery`).
#[must_use]
pub fn build_mgp_error_data(recovery: Option<MgpErrorRecovery>) -> Value {
    match recovery {
        Some(rec) => serde_json::json!({
            "_mgp": serde_json::to_value(&rec).unwrap_or_default()
        }),
        None => serde_json::json!({ "_mgp": {} }),
    }
}

// ============================================================
// Code Safety Level (§7.2)
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CodeSafetyLevel {
    Unrestricted,
    Standard,
    Strict,
    Readonly,
}

impl CodeSafetyLevel {
    /// Parse a code safety level string. Unknown values map to `Standard`.
    #[must_use]
    pub fn from_str_lossy(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "unrestricted" => Self::Unrestricted,
            "strict" => Self::Strict,
            "readonly" => Self::Readonly,
            _ => Self::Standard,
        }
    }
}

/// Code safety metadata attached to tool security (§7.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeSafetyMetadata {
    pub level: CodeSafetyLevel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_code_size_bytes: Option<usize>,
}

// ============================================================
// Risk Level (§4)
// ============================================================

/// Tool risk level derived from trust, validator, and permissions.
/// Ordering: Safe < Moderate < Dangerous (max() yields highest risk).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    Safe,
    Moderate,
    Dangerous,
}

// ============================================================
// Trust Level (§2, §4)
// ============================================================

/// Server trust level — determined by kernel (mcp.toml config), not self-declared.
/// Ordering: Untrusted < Experimental < Standard < Core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrustLevel {
    Untrusted,
    Experimental,
    Standard,
    Core,
}

impl TrustLevel {
    /// Parse a trust level string. Unknown values map to `Untrusted`.
    #[must_use]
    pub fn from_str_lossy(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "core" => Self::Core,
            "standard" => Self::Standard,
            "experimental" => Self::Experimental,
            _ => Self::Untrusted,
        }
    }
}

// ============================================================
// Capability Negotiation Types (§2)
// ============================================================

/// MGP capabilities sent by kernel in `initialize` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MgpClientCapabilities {
    pub version: String,
    pub extensions: Vec<String>,
}

/// MGP capabilities returned by server in `initialize` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MgpServerCapabilities {
    pub version: String,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub server_id: Option<String>,
    #[serde(default)]
    pub trust_level: Option<String>,
    /// Permissions the server declares it requires (§3).
    #[serde(default)]
    pub permissions_required: Vec<String>,
}

/// Result of MGP capability negotiation, stored in `McpServerHandle`.
#[derive(Debug, Clone)]
pub struct NegotiatedMgp {
    pub version: String,
    pub active_extensions: Vec<String>,
    pub trust_level: TrustLevel,
}

// ============================================================
// Tool Security Metadata (§4)
// ============================================================

/// Security metadata injected into tool schemas at collection time.
#[derive(Debug, Clone, Serialize)]
pub struct ToolSecurityMetadata {
    pub effective_risk_level: RiskLevel,
    pub trust_level: TrustLevel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_safety: Option<CodeSafetyMetadata>,
}

// ============================================================
// mcp.toml Config Extension
// ============================================================

/// Optional `[servers.mgp]` section in mcp.toml.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MgpServerConfig {
    /// Expected trust level for this server (kernel-authoritative).
    #[serde(default)]
    pub trust_level: Option<String>,
}

// ============================================================
// Negotiation Logic
// ============================================================

/// Check semver compatibility per §2.4 rule 4 and §2.5.
/// Pre-1.0: minor version changes MAY contain breaking changes (warn but allow).
/// Post-1.0: major must match, minor is backward compatible.
#[must_use]
fn semver_compatible(client_ver: &str, server_ver: &str) -> bool {
    let parse = |v: &str| -> Option<(u64, u64)> {
        let v = v.split('-').next().unwrap_or(v); // strip pre-release
        let mut parts = v.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        Some((major, minor))
    };
    match (parse(client_ver), parse(server_ver)) {
        (Some((cm, _)), Some((sm, _))) if cm >= 1 || sm >= 1 => cm == sm,
        (Some((cm, cmin)), Some((sm, smin))) => cm == sm && cmin == smin,
        _ => true, // unparseable → allow
    }
}

/// Negotiate MGP capabilities between kernel and server.
///
/// - `server_caps`: parsed from server's `initialize` response (None if server doesn't support MGP)
/// - `config_trust`: trust level from mcp.toml `[servers.mgp]` (kernel-authoritative)
///
/// Returns `None` if `server_caps` is `None` (standard MCP server).
#[must_use]
pub fn negotiate(
    server_caps: Option<&MgpServerCapabilities>,
    config_trust: Option<&str>,
) -> Option<NegotiatedMgp> {
    let server = server_caps?;

    // §2.4 rule 4: semver compatibility — major must match, minor is backward compatible
    if !semver_compatible(MGP_VERSION, &server.version) {
        tracing::warn!(
            client_version = %MGP_VERSION,
            server_version = %server.version,
            "MGP version mismatch — connection may have reduced functionality"
        );
    }

    // Extension intersection: only activate extensions both sides support
    let active_extensions: Vec<String> = CLIENT_EXTENSIONS
        .iter()
        .filter(|ext| server.extensions.iter().any(|s| s == *ext))
        .map(|s| (*s).to_string())
        .collect();

    // Trust level: mcp.toml config > server self-declaration > default Untrusted
    let trust_level = if let Some(cfg) = config_trust {
        TrustLevel::from_str_lossy(cfg)
    } else if let Some(ref srv) = server.trust_level {
        TrustLevel::from_str_lossy(srv)
    } else {
        TrustLevel::Untrusted
    };

    Some(NegotiatedMgp {
        version: server.version.clone(),
        active_extensions,
        trust_level,
    })
}

// ============================================================
// effective_risk_level Derivation (§4.6)
// ============================================================

/// Permission risk classification for `derive_effective_risk_level` (§4.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionRiskClass {
    /// No dangerous permissions (default)
    Safe,
    /// Requires `filesystem.write` or similar moderate permissions
    Moderate,
    /// Requires `shell.execute`, `code_execution`, or other dangerous permissions
    Dangerous,
}

impl PermissionRiskClass {
    /// Classify a set of permission strings per §4.6.
    #[must_use]
    pub fn from_permissions(permissions: &[String]) -> Self {
        let dangerous = ["shell.execute", "code_execution"];
        let moderate = ["filesystem.write"];
        if permissions.iter().any(|p| dangerous.contains(&p.as_str())) {
            Self::Dangerous
        } else if permissions.iter().any(|p| moderate.contains(&p.as_str())) {
            Self::Moderate
        } else {
            Self::Safe
        }
    }
}

/// Derive `effective_risk_level = max(trust_derived, validator_derived, permissions_derived)`.
///
/// - `trust`: server trust level → risk mapping (§4.6)
/// - `validator`: kernel-side validator name (e.g., "sandbox") → risk mapping (§4.6)
/// - `permissions_class`: risk class derived from `permissions_required` (§4.6)
#[must_use]
pub fn derive_effective_risk_level(
    trust: TrustLevel,
    validator: Option<&str>,
    permissions_class: PermissionRiskClass,
) -> RiskLevel {
    // Trust level → base risk (§4.6)
    let trust_risk = match trust {
        TrustLevel::Core => RiskLevel::Safe,
        TrustLevel::Standard => RiskLevel::Moderate,
        TrustLevel::Experimental | TrustLevel::Untrusted => RiskLevel::Dangerous,
    };

    // Validator → risk (§4.6)
    let validator_risk = match validator {
        Some("readonly") => RiskLevel::Safe,
        Some("sandbox") | Some("network_restricted") | Some("code_safety") => RiskLevel::Moderate,
        Some("none") => RiskLevel::Dangerous,
        _ => RiskLevel::Safe,
    };

    // Permissions → risk (§4.6)
    let permission_risk = match permissions_class {
        PermissionRiskClass::Dangerous => RiskLevel::Dangerous,
        PermissionRiskClass::Moderate => RiskLevel::Moderate,
        PermissionRiskClass::Safe => RiskLevel::Safe,
    };

    // max() works because RiskLevel derives Ord: Safe < Moderate < Dangerous
    trust_risk.max(validator_risk).max(permission_risk)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn risk_level_ordering() {
        assert!(RiskLevel::Safe < RiskLevel::Moderate);
        assert!(RiskLevel::Moderate < RiskLevel::Dangerous);
    }

    #[test]
    fn trust_level_ordering() {
        assert!(TrustLevel::Untrusted < TrustLevel::Experimental);
        assert!(TrustLevel::Experimental < TrustLevel::Standard);
        assert!(TrustLevel::Standard < TrustLevel::Core);
    }

    #[test]
    fn derive_risk_core_no_validator() {
        assert_eq!(
            derive_effective_risk_level(TrustLevel::Core, None, PermissionRiskClass::Safe),
            RiskLevel::Safe
        );
    }

    #[test]
    fn derive_risk_standard_is_moderate() {
        // §4.6: Standard → Moderate
        assert_eq!(
            derive_effective_risk_level(TrustLevel::Standard, None, PermissionRiskClass::Safe),
            RiskLevel::Moderate
        );
    }

    #[test]
    fn derive_risk_experimental_sandbox() {
        // max(Dangerous, Moderate, Safe) = Dangerous
        assert_eq!(
            derive_effective_risk_level(TrustLevel::Experimental, Some("sandbox"), PermissionRiskClass::Safe),
            RiskLevel::Dangerous
        );
    }

    #[test]
    fn derive_risk_untrusted_always_dangerous() {
        assert_eq!(
            derive_effective_risk_level(TrustLevel::Untrusted, None, PermissionRiskClass::Safe),
            RiskLevel::Dangerous
        );
    }

    #[test]
    fn derive_risk_validator_mappings() {
        // §4.6: readonly→Safe, sandbox→Moderate, none→Dangerous
        assert_eq!(
            derive_effective_risk_level(TrustLevel::Core, Some("readonly"), PermissionRiskClass::Safe),
            RiskLevel::Safe
        );
        assert_eq!(
            derive_effective_risk_level(TrustLevel::Core, Some("sandbox"), PermissionRiskClass::Safe),
            RiskLevel::Moderate
        );
        assert_eq!(
            derive_effective_risk_level(TrustLevel::Core, Some("network_restricted"), PermissionRiskClass::Safe),
            RiskLevel::Moderate
        );
        assert_eq!(
            derive_effective_risk_level(TrustLevel::Core, Some("code_safety"), PermissionRiskClass::Safe),
            RiskLevel::Moderate
        );
        assert_eq!(
            derive_effective_risk_level(TrustLevel::Core, Some("none"), PermissionRiskClass::Safe),
            RiskLevel::Dangerous
        );
    }

    #[test]
    fn derive_risk_dangerous_permissions_override() {
        // Even Core trust: dangerous permissions → Dangerous
        assert_eq!(
            derive_effective_risk_level(TrustLevel::Core, None, PermissionRiskClass::Dangerous),
            RiskLevel::Dangerous
        );
    }

    #[test]
    fn derive_risk_moderate_permissions() {
        // §4.6: filesystem.write → Moderate
        assert_eq!(
            derive_effective_risk_level(TrustLevel::Core, None, PermissionRiskClass::Moderate),
            RiskLevel::Moderate
        );
    }

    #[test]
    fn permission_risk_class_from_permissions() {
        assert_eq!(
            PermissionRiskClass::from_permissions(&["shell.execute".into()]),
            PermissionRiskClass::Dangerous
        );
        assert_eq!(
            PermissionRiskClass::from_permissions(&["filesystem.write".into()]),
            PermissionRiskClass::Moderate
        );
        assert_eq!(
            PermissionRiskClass::from_permissions(&["memory.read".into()]),
            PermissionRiskClass::Safe
        );
    }

    #[test]
    fn semver_compatible_same_version() {
        assert!(semver_compatible("0.6.0", "0.6.0"));
    }

    #[test]
    fn semver_compatible_different_minor_pre1() {
        // Pre-1.0: different minor is incompatible
        assert!(!semver_compatible("0.6.0", "0.5.0"));
    }

    #[test]
    fn semver_compatible_same_minor_pre1() {
        assert!(semver_compatible("0.6.0", "0.6.1"));
    }

    #[test]
    fn negotiate_none_caps_returns_none() {
        assert!(negotiate(None, None).is_none());
    }

    #[test]
    fn negotiate_with_matching_extensions() {
        let server = MgpServerCapabilities {
            version: "0.6.0".to_string(),
            extensions: vec!["tool_security".to_string(), "permissions".to_string()],
            server_id: Some("test".to_string()),
            trust_level: Some("standard".to_string()),
            permissions_required: Vec::new(),
        };
        let result = negotiate(Some(&server), None).unwrap();
        // Tier 2: both tool_security and permissions are now in CLIENT_EXTENSIONS
        assert!(result.active_extensions.contains(&"tool_security".to_string()));
        assert!(result.active_extensions.contains(&"permissions".to_string()));
        // No config_trust → falls through to server declaration
        assert_eq!(result.trust_level, TrustLevel::Standard);
    }

    #[test]
    fn negotiate_config_trust_overrides_server() {
        let server = MgpServerCapabilities {
            version: "0.6.0".to_string(),
            extensions: vec!["tool_security".to_string()],
            server_id: None,
            trust_level: Some("core".to_string()),
            permissions_required: Vec::new(),
        };
        // Config says experimental, server says core → config wins
        let result = negotiate(Some(&server), Some("experimental")).unwrap();
        assert_eq!(result.trust_level, TrustLevel::Experimental);
    }

    #[test]
    fn trust_level_from_str_unknown_is_untrusted() {
        assert_eq!(TrustLevel::from_str_lossy("unknown"), TrustLevel::Untrusted);
        assert_eq!(TrustLevel::from_str_lossy(""), TrustLevel::Untrusted);
    }

    #[test]
    fn build_mgp_error_data_without_recovery() {
        let data = build_mgp_error_data(None);
        assert!(data["_mgp"].is_object());
        // No recovery fields when None
        assert!(data["_mgp"].get("retryable").is_none());
    }

    #[test]
    fn build_mgp_error_data_with_recovery() {
        let recovery = MgpErrorRecovery {
            category: "permission".to_string(),
            retryable: true,
            retry_after_ms: Some(5000),
            retry_strategy: Some("exponential".to_string()),
            max_retries: Some(3),
            fallback_tool: None,
            details: None,
        };
        let data = build_mgp_error_data(Some(recovery));
        // §14.4: recovery fields directly under _mgp
        assert_eq!(data["_mgp"]["retryable"], true);
        assert_eq!(data["_mgp"]["retry_after_ms"], 5000);
        assert_eq!(data["_mgp"]["category"], "permission");
    }

    #[test]
    fn code_safety_level_from_str_lossy() {
        assert_eq!(CodeSafetyLevel::from_str_lossy("unrestricted"), CodeSafetyLevel::Unrestricted);
        assert_eq!(CodeSafetyLevel::from_str_lossy("strict"), CodeSafetyLevel::Strict);
        assert_eq!(CodeSafetyLevel::from_str_lossy("readonly"), CodeSafetyLevel::Readonly);
        assert_eq!(CodeSafetyLevel::from_str_lossy("unknown"), CodeSafetyLevel::Standard);
        assert_eq!(CodeSafetyLevel::from_str_lossy(""), CodeSafetyLevel::Standard);
    }

    #[test]
    fn permissions_required_deserialize_default() {
        let json = r#"{"version":"0.6.0"}"#;
        let caps: MgpServerCapabilities = serde_json::from_str(json).unwrap();
        assert!(caps.permissions_required.is_empty());
    }

    #[test]
    fn permissions_required_deserialize_with_values() {
        let json = r#"{"version":"0.6.0","permissions_required":["filesystem","network"]}"#;
        let caps: MgpServerCapabilities = serde_json::from_str(json).unwrap();
        assert_eq!(caps.permissions_required, vec!["filesystem", "network"]);
    }

    #[test]
    fn client_extensions_includes_tier2() {
        assert!(CLIENT_EXTENSIONS.contains(&"permissions"));
        assert!(CLIENT_EXTENSIONS.contains(&"access_control"));
        assert!(CLIENT_EXTENSIONS.contains(&"audit"));
        assert!(CLIENT_EXTENSIONS.contains(&"code_safety"));
        assert!(CLIENT_EXTENSIONS.contains(&"error_handling"));
        // Tier 1 extension still present
        assert!(CLIENT_EXTENSIONS.contains(&"tool_security"));
    }

    #[test]
    fn client_extensions_includes_tier3() {
        assert!(CLIENT_EXTENSIONS.contains(&"lifecycle"));
        assert!(CLIENT_EXTENSIONS.contains(&"streaming"));
        assert!(CLIENT_EXTENSIONS.contains(&"progress"));
        assert!(CLIENT_EXTENSIONS.contains(&"events"));
        assert!(CLIENT_EXTENSIONS.contains(&"callbacks"));
        assert!(CLIENT_EXTENSIONS.contains(&"discovery"));
        assert!(CLIENT_EXTENSIONS.contains(&"tool_discovery"));
        assert_eq!(CLIENT_EXTENSIONS.len(), 13);
    }
}
