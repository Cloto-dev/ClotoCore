use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use serde::Deserialize;
use std::{collections::HashMap, sync::Arc};
use tracing::{error, info};

use crate::managers::McpClientManager;
use crate::{AppError, AppResult, AppState};

use super::{check_auth, ok_data, spawn_admin_audit};

#[derive(Debug, Deserialize)]
pub struct PluginToggleRequest {
    pub id: String,
    pub is_active: bool,
}

#[derive(Deserialize)]
pub struct UpdateConfigPayload {
    pub key: String,
    pub value: String,
}

#[derive(Deserialize)]
pub struct GrantPermissionRequest {
    pub permission: cloto_shared::Permission,
}

#[derive(Deserialize)]
pub struct RevokePermissionRequest {
    pub permission: cloto_shared::Permission,
}

/// List all registered plugins with their current settings.
///
/// **Route:** `GET /api/plugins`
///
/// # Authentication
/// No authentication required (read-only).
///
/// # Response
/// Returns a JSON array of plugin manifests merged with database settings
/// (enabled/disabled state, configuration overrides).
///
/// Each entry includes: `id`, `name`, `description`, `version`, `category`,
/// `tags`, `capabilities`, `is_active`, and `provided_tools`.
pub async fn get_plugins(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let manifests = state
        .plugin_manager
        .list_plugins_with_settings(&state.registry)
        .await?;
    ok_data(serde_json::json!(manifests))
}

/// Get plugin configuration values.
///
/// **Route:** `GET /api/plugins/:id/config`
///
/// # Authentication
/// Requires valid API key in `X-API-Key` header.
/// Config may contain sensitive values (API keys, tokens).
///
/// # Response
/// - **200 OK:** JSON object of key-value configuration pairs
/// - **403 Forbidden:** Invalid or missing API key
pub async fn get_plugin_config(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let config = state.plugin_manager.get_config(&id).await?;
    ok_data(serde_json::json!(config))
}

/// Update a single plugin configuration key-value pair.
///
/// **Route:** `POST /api/plugins/:id/config`
///
/// # Authentication
/// Requires valid API key in `X-API-Key` header.
///
/// # Request Body
/// ```json
/// { "key": "api_key", "value": "your-api-key" }
/// ```
///
/// # Side Effects
/// - Broadcasts `ConfigUpdated` event to all subscribers
/// - Writes audit log entry with actor, target, and trace ID
///
/// # Response
/// - **200 OK:** `{ "status": "success" }`
/// - **403 Forbidden:** Invalid or missing API key
pub async fn update_plugin_config(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<UpdateConfigPayload>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    state
        .plugin_manager
        .update_config(&id, &payload.key, &payload.value)
        .await?;

    info!(plugin_id = %id, key = %payload.key, "⚙️ Config updated for plugin. Broadcasting update...");

    // Get latest settings and notify
    if let Ok(full_config) = state.plugin_manager.get_config(&id).await {
        // bug-462: the event is broadcast to every SSE subscriber and cached in
        // the replay history, so a plaintext api_key/token here would leak to
        // any authenticated client. Mask sensitive values the same way
        // get_mcp_server_settings does before it leaves the handler.
        let masked_config: std::collections::HashMap<String, String> = full_config
            .into_iter()
            .map(|(k, v)| {
                let upper = k.to_uppercase();
                let is_secret = upper.contains("KEY")
                    || upper.contains("SECRET")
                    || upper.contains("TOKEN")
                    || upper.contains("PASSWORD")
                    || upper.contains("CREDENTIAL");
                (k, if is_secret { "***".to_string() } else { v })
            })
            .collect();
        let envelope = crate::EnvelopedEvent::system(cloto_shared::ClotoEventData::ConfigUpdated {
            plugin_id: id.clone(),
            config: masked_config,
        });
        let event = envelope.event.clone();
        // H-04: Log send errors instead of silently ignoring
        if let Err(e) = state.event_tx.send(envelope).await {
            error!("Failed to send config update event: {}", e);
        }

        spawn_admin_audit(
            state.pool.clone(),
            "CONFIG_UPDATED",
            id.clone(),
            format!("Configuration key '{}' updated", payload.key),
            None,
            Some(serde_json::json!({ "key": payload.key, "value_length": payload.value.len() })),
            Some(event.trace_id.to_string()),
        );
    }

    ok_data(serde_json::json!({}))
}

/// Batch apply plugin enabled/disabled settings.
///
/// **Route:** `POST /api/plugins/apply`
///
/// # Authentication
/// Requires valid API key in `X-API-Key` header.
///
/// # Request Body
/// ```json
/// [
///   { "id": "plugin-1", "is_active": true },
///   { "id": "plugin-2", "is_active": false }
/// ]
/// ```
///
/// # Response
/// - **200 OK:** `true` on success
/// - **403 Forbidden:** Invalid or missing API key
pub async fn apply_plugin_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<Vec<PluginToggleRequest>>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    info!(
        count = payload.len(),
        "📥 Received plugin settings apply request"
    );

    let settings = payload.into_iter().map(|i| (i.id, i.is_active)).collect();

    state.plugin_manager.apply_settings(settings).await?;
    ok_data(serde_json::json!({}))
}

/// Grant a permission to a plugin.
///
/// **Route:** `POST /api/plugins/:id/permissions/grant`
///
/// # Authentication
/// Requires valid API key in `X-API-Key` header.
///
/// # Request Body
/// ```json
/// { "permission": "NetworkAccess" }
/// ```
///
/// Valid permissions: `NetworkAccess`, `FileRead`, `FileWrite`,
/// `ProcessExecution`, `VisionRead`, `AdminAccess`.
///
/// # Side Effects
/// - Broadcasts `PermissionGranted` event (triggers capability injection)
/// - Writes audit log entry
///
/// # Response
/// - **200 OK:** `{ "status": "success" }`
/// - **403 Forbidden:** Invalid or missing API key
pub async fn grant_permission_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<GrantPermissionRequest>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    info!(
        plugin_id = %id,
        permission = ?payload.permission,
        "🔐 Granting permission to plugin"
    );

    state
        .plugin_manager
        .grant_permission(&id, payload.permission.clone())
        .await?;

    // Notify the event loop so it injects the capability.
    let envelope = crate::EnvelopedEvent::system(cloto_shared::ClotoEventData::PermissionGranted {
        plugin_id: id.clone(),
        permission: payload.permission.to_string(),
    });
    let event = envelope.event.clone();
    // H-04: Log send errors instead of silently ignoring
    if let Err(e) = state.event_tx.send(envelope).await {
        error!("Failed to send permission grant event: {}", e);
    }

    spawn_admin_audit(
        state.pool.clone(),
        "PERMISSION_GRANTED",
        id.clone(),
        "Administrator approved permission request".to_string(),
        Some(format!("{:?}", payload.permission)),
        None,
        Some(event.trace_id.to_string()),
    );

    ok_data(serde_json::json!({}))
}

/// Get the current effective permissions for a plugin.
///
/// **Route:** `GET /api/plugins/:id/permissions`
pub async fn get_plugin_permissions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let perms = state.plugin_manager.get_permissions(&id).await?;
    let list: Vec<String> = perms.iter().map(|p| format!("{:?}", p)).collect();
    ok_data(serde_json::json!({ "plugin_id": id, "permissions": list }))
}

/// Revoke a permission from a plugin.
///
/// **Route:** `DELETE /api/plugins/:id/permissions`
///
/// # Authentication
/// Requires valid API key in `X-API-Key` header.
///
/// # Request Body
/// ```json
/// { "permission": "NetworkAccess" }
/// ```
pub async fn revoke_permission_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<RevokePermissionRequest>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    info!(plugin_id = %id, permission = ?payload.permission, "🔓 Revoking permission from plugin");

    state
        .plugin_manager
        .revoke_permission(&id, &payload.permission, &state.registry)
        .await?;

    spawn_admin_audit(
        state.pool.clone(),
        "PERMISSION_REVOKED",
        id.clone(),
        "Administrator revoked permission".to_string(),
        Some(format!("{:?}", payload.permission)),
        None,
        None,
    );

    ok_data(serde_json::json!({}))
}

// ============================================================
// MCP Dynamic Server Management
// ============================================================

/// Generate, validate, and write a dynamic MCP server Python script.
fn generate_mcp_script(
    name: &str,
    code: &str,
    description: &str,
) -> AppResult<(String, Vec<String>, String)> {
    // Validate code safety before writing to disk
    if let Err(violations) = crate::managers::mcp_tool_validator::validate_mcp_code(
        code,
        crate::managers::mcp_mgp::CodeSafetyLevel::Standard,
    ) {
        return Err(AppError::Validation(format!(
            "Code validation failed: {}",
            violations.join("; ")
        )));
    }

    let script = format!(
        r#""""MCP Server: {name} — {description}"""
from mcp.server import Server
from mcp.server.stdio import stdio_server

app = Server("{name}")

{code}

async def main():
    async with stdio_server() as (read, write):
        await app.run(read, write)

if __name__ == "__main__":
    import asyncio
    asyncio.run(main())
"#,
        name = name,
        description = description.replace('"', r#"\""#),
        code = code,
    );

    let script_filename = crate::managers::mcp_types::mcp_script_filename(name);
    let scripts_dir = std::path::Path::new(crate::managers::mcp_types::MCP_SCRIPTS_DIR);
    if !scripts_dir.exists() {
        std::fs::create_dir_all(scripts_dir).map_err(|e| {
            AppError::Internal(anyhow::anyhow!(
                "Failed to create {} directory: {}",
                crate::managers::mcp_types::MCP_SCRIPTS_DIR,
                e
            ))
        })?;
    }
    std::fs::write(crate::managers::mcp_types::mcp_script_path(name), &script).map_err(|e| {
        AppError::Internal(anyhow::anyhow!("Failed to write MCP server script: {}", e))
    })?;

    let python = if cfg!(windows) { "python" } else { "python3" };
    Ok((
        python.to_string(),
        vec![format!(
            "{}/{}",
            crate::managers::mcp_types::MCP_SCRIPTS_DIR,
            script_filename
        )],
        script,
    ))
}

/// Validate a dynamic MCP server name.
///
/// Accepts ASCII alphanumerics plus `_`, `-`, and `.`. Dots stay legal for
/// third-party dotted ids even though first-party ids are bare since the
/// category-prefix retirement (docs/CATEGORY_PREFIX_RETIREMENT_DESIGN.md).
/// Rejects anything that could form a traversal-like path fragment
/// (leading/trailing dot or a `..` sequence).
fn validate_server_name(name: &str) -> AppResult<()> {
    if name.is_empty() || name.len() > 64 {
        return Err(AppError::Validation(
            "Server name must be 1-64 characters".into(),
        ));
    }
    if name.starts_with('.')
        || name.ends_with('.')
        || name.contains("..")
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(AppError::Validation(
            "Server name must contain only alphanumeric characters, underscores, hyphens, and dots (no leading/trailing dot and no '..' sequence)"
                .into(),
        ));
    }
    Ok(())
}

fn remote_server_config(
    name: &str,
    body: &serde_json::Value,
) -> AppResult<Option<crate::managers::mcp_protocol::McpServerConfig>> {
    let transport = body
        .get("transport")
        .and_then(|v| v.as_str())
        .unwrap_or("stdio");
    if transport == "stdio" {
        return Ok(None);
    }
    if transport != "streamable-http" {
        return Err(AppError::Validation(format!(
            "Unsupported MCP transport: {transport}"
        )));
    }
    if body.get("code").is_some() || body.get("command").is_some() || body.get("args").is_some() {
        return Err(AppError::Validation(
            "streamable-http servers accept 'url' instead of 'code', 'command', or 'args'".into(),
        ));
    }
    let url = body
        .get("url")
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            AppError::Validation("Missing required field for streamable-http server: url".into())
        })?;

    Ok(Some(crate::managers::mcp_protocol::McpServerConfig {
        id: name.to_string(),
        transport: transport.to_string(),
        url: Some(url.to_string()),
        auth_token: body
            .get("auth_token")
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
            .map(str::to_string),
        auto_restart: Some(
            body.get("auto_restart")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true),
        ),
        display_name: body
            .get("display_name")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        ..Default::default()
    }))
}

pub async fn create_mcp_server(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Validation("Missing required field: name".into()))?;

    validate_server_name(name)?;

    if let Some(config) = remote_server_config(name, &body)? {
        let description = body
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from);
        let tool_names = state
            .mcp_manager
            .add_server_config(config, None, description)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to add MCP server: {}", e)))?;

        tracing::info!(name = %name, tools = ?tool_names, "Remote MCP server added");
        return ok_data(serde_json::json!({
            "name": name,
            "tools": tool_names,
        }));
    }

    // Determine command/args: either explicit or auto-generated from code
    let (command, args, script_content) =
        if let Some(code) = body.get("code").and_then(|v| v.as_str()) {
            let description = body
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("A dynamically generated MCP server.");
            let (command, args, script) = generate_mcp_script(name, code, description)?;
            (command, args, Some(script))
        } else {
            // Explicit command/args
            let command = body
                .get("command")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AppError::Validation("Missing 'command' or 'code' field".into()))?
                .to_string();

            let args: Vec<String> = body
                .get("args")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            (command, args, None)
        };

    // Add server via McpClientManager (handles connection + DB persistence).
    // Dynamic servers added via this API path have no upfront seal — the
    // kernel applies v0.6.3 §10 inv 3 force-untrusted on connect.
    let tool_names = state
        .mcp_manager
        .add_server(
            name.to_string(),
            command.clone(),
            args.clone(),
            script_content,
            body.get("description")
                .and_then(|v| v.as_str())
                .map(String::from),
            None,
            None,
            std::collections::HashMap::new(),
        )
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to add MCP server: {}", e)))?;

    tracing::info!(name = %name, tools = ?tool_names, "🔌 Dynamic MCP server added");

    ok_data(serde_json::json!({
        "name": name,
        "tools": tool_names,
    }))
}

pub async fn list_mcp_servers(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let servers = state.mcp_manager.list_servers().await;

    ok_data(serde_json::json!({
        "servers": servers,
        "count": servers.len(),
    }))
}

pub async fn delete_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    // Remove from McpClientManager (handles disconnect + DB deletion)
    state
        .mcp_manager
        .remove_server(&name)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;

    // Remove the auto-generated script, through the same helper that writes it.
    // A failure here is reported rather than swallowed: the file holds
    // user-supplied Python, and "the server is gone" must not quietly mean
    // "its code is still on disk".
    match crate::managers::mcp_types::remove_mcp_script(&name) {
        Ok(true) => tracing::debug!(name = %name, "Removed generated MCP server script"),
        Ok(false) => {}
        Err(e) => tracing::warn!(
            name = %name,
            error = %e,
            path = %crate::managers::mcp_types::mcp_script_path(&name).display(),
            "MCP server removed but its generated script could not be deleted"
        ),
    }

    tracing::info!(name = %name, "🗑️ MCP server removed");

    ok_data(serde_json::json!({
        "name": name,
    }))
}

// ============================================================
// MCP Server Settings & Access Control (MCP_SERVER_UI_DESIGN.md §4)
// ============================================================

/// GET /api/mcp/servers/:name/settings
pub async fn get_mcp_server_settings(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let settings = crate::db::get_mcp_server_settings(&state.pool, &name)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;

    // Get in-memory config env (from mcp.toml or runtime) as defaults
    let config_env = state.mcp_manager.get_server_env(&name).await;

    if let Some(record) = settings {
        // Merge: in-memory config env as base, DB env overrides
        let db_env: HashMap<String, String> = serde_json::from_str(&record.env).unwrap_or_default();
        let mut merged = config_env;
        for (k, v) in &db_env {
            merged.insert(k.clone(), v.clone());
        }
        // Mask only sensitive values (KEY, SECRET, TOKEN, PASSWORD)
        let masked_env: HashMap<String, String> = merged
            .iter()
            .map(|(k, v)| {
                let upper = k.to_uppercase();
                let is_secret = upper.contains("KEY")
                    || upper.contains("SECRET")
                    || upper.contains("TOKEN")
                    || upper.contains("PASSWORD")
                    || upper.contains("CREDENTIAL");
                (
                    k.clone(),
                    if is_secret {
                        "***".to_string()
                    } else {
                        v.clone()
                    },
                )
            })
            .collect();

        ok_data(serde_json::json!({
            "server_id": record.name,
            "default_policy": record.default_policy,
            "config": {},
            "env": masked_env,
            "auto_restart": record.auto_restart,
            "transport": record.transport,
            "url": record.url,
            "auth_token_configured": record.auth_token.is_some(),
            "command": record.command,
            "args": serde_json::from_str::<Vec<String>>(&record.args).unwrap_or_default(),
            "description": record.description,
        }))
    } else {
        Err(AppError::Validation(format!(
            "MCP server '{}' not found",
            name
        )))
    }
}

/// PUT /api/mcp/servers/:name/settings
#[allow(clippy::too_many_lines)]
pub async fn update_mcp_server_settings(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    if let Some(policy) = body.get("default_policy").and_then(|v| v.as_str()) {
        if !["opt-in", "opt-out"].contains(&policy) {
            return Err(AppError::Validation(
                "default_policy must be 'opt-in' or 'opt-out'".into(),
            ));
        }
        let rows = crate::db::update_mcp_server_default_policy(&state.pool, &name, policy)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;

        if rows == 0 {
            return Err(AppError::Validation(format!(
                "MCP server '{}' not found",
                name
            )));
        }
    }

    // Handle env updates
    if let Some(env_obj) = body.get("env").and_then(|v| v.as_object()) {
        // Load existing env from DB to preserve unchanged values (sent as "***")
        let existing_env: HashMap<String, String> = if let Ok(Some(record)) =
            crate::db::get_mcp_server_settings(&state.pool, &name).await
        {
            serde_json::from_str(&record.env).unwrap_or_default()
        } else {
            HashMap::new()
        };

        let mut merged_env: HashMap<String, String> = HashMap::new();
        for (key, value) in env_obj {
            if let Some(val_str) = value.as_str() {
                if val_str == "***" {
                    // Preserve existing value
                    if let Some(existing_val) = existing_env.get(key) {
                        merged_env.insert(key.clone(), existing_val.clone());
                    }
                } else if !val_str.is_empty() {
                    // New or updated value
                    merged_env.insert(key.clone(), val_str.to_string());
                }
                // Empty string = remove the key (omit from merged_env)
            }
        }

        // Ensure server is in DB before updating env
        let rows = crate::db::update_mcp_server_env(
            &state.pool,
            &name,
            &serde_json::to_string(&merged_env).unwrap_or_else(|_| "{}".to_string()),
        )
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;

        if rows == 0 {
            tracing::warn!("env update found no DB row for server '{}'", name);
        }

        // Update in-memory config and restart server
        if let Err(e) = state.mcp_manager.update_server_env(&name, merged_env).await {
            tracing::warn!("Failed to restart server after env update: {}", e);
        }
    }

    spawn_admin_audit(
        state.pool.clone(),
        "MCP_SERVER_SETTINGS_UPDATED",
        name.clone(),
        "MCP server settings updated".to_string(),
        None,
        None,
        None,
    );

    ok_data(serde_json::json!({}))
}

/// GET /api/mcp/servers/:name/access
pub async fn get_mcp_server_access(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let entries = crate::db::get_access_entries_for_server(&state.pool, &name)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;

    // Get server's default_policy
    let settings = crate::db::get_mcp_server_settings(&state.pool, &name)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;

    let default_policy = settings
        .as_ref()
        .map_or("opt-in", |r| r.default_policy.as_str());

    // Get tools from running server
    let tools: Vec<String> = {
        let servers = state.mcp_manager.list_servers().await;
        servers
            .iter()
            .find(|s| s.id == name)
            .map(|s| s.tools.clone())
            .unwrap_or_default()
    };

    ok_data(serde_json::json!({
        "server_id": name,
        "default_policy": default_policy,
        "tools": tools,
        "entries": entries,
    }))
}

/// PUT /api/mcp/servers/:name/access
pub async fn put_mcp_server_access(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let entries_val = body
        .get("entries")
        .ok_or_else(|| AppError::Validation("Missing required field: entries".into()))?;

    let entries: Vec<crate::db::AccessControlEntry> =
        serde_json::from_value(entries_val.clone())
            .map_err(|e| AppError::Validation(format!("Invalid entries format: {}", e)))?;

    // Validate all entries reference this server
    for entry in &entries {
        if entry.server_id != name {
            return Err(AppError::Validation(format!(
                "Entry server_id '{}' does not match route server '{}'",
                entry.server_id, name
            )));
        }
        if entry.entry_type == crate::db::mcp::EntryType::Capability {
            return Err(AppError::Validation(
                "Cannot bulk-update capability entries; only server_grant and tool_grant allowed"
                    .into(),
            ));
        }
    }

    crate::db::put_access_entries(&state.pool, &name, &entries)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;

    spawn_admin_audit(
        state.pool.clone(),
        "MCP_ACCESS_UPDATED",
        name.clone(),
        format!("Access control updated with {} entries", entries.len()),
        None,
        None,
        None,
    );

    ok_data(serde_json::json!({
        "count": entries.len(),
    }))
}

/// GET /api/mcp/access/by-agent/:agent_id
pub async fn get_agent_access(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let entries = crate::db::get_access_entries_for_agent(&state.pool, &agent_id)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;

    ok_data(serde_json::json!({
        "agent_id": agent_id,
        "entries": entries,
    }))
}

/// PUT /api/agents/:agent_id/mcp-access
///
/// Replace all `server_grant` entries for the given agent in a single call.
/// Preserves `tool_grant` and `capability` entries. Used by the dashboard's
/// agent-centric flows (AgentPluginWorkspace, SetupWizard, AgentTerminal
/// import) to avoid the 2N REST-call pattern that used to trip the rate
/// limiter on bulk changes.
///
/// Body: `{ "granted_server_ids": ["terminal", "mind.cerebras", ...] }`
pub async fn put_agent_mcp_access(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let ids_val = body
        .get("granted_server_ids")
        .ok_or_else(|| AppError::Validation("Missing required field: granted_server_ids".into()))?;

    let granted_server_ids: Vec<String> = serde_json::from_value(ids_val.clone())
        .map_err(|e| AppError::Validation(format!("Invalid granted_server_ids format: {}", e)))?;

    crate::db::put_agent_server_grants(&state.pool, &agent_id, &granted_server_ids, "admin")
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;

    let count = granted_server_ids.len();
    spawn_admin_audit(
        state.pool.clone(),
        "AGENT_MCP_ACCESS_UPDATED",
        agent_id.clone(),
        format!("Agent MCP access updated with {} server grants", count),
        None,
        Some(serde_json::json!({ "granted_server_ids": granted_server_ids })),
        None,
    );

    ok_data(serde_json::json!({ "count": count }))
}

async fn server_lifecycle(
    state: &Arc<AppState>,
    name: &str,
    action: &str,
    audit_event: &str,
    tools: Result<Option<Vec<String>>, anyhow::Error>,
) -> AppResult<Json<serde_json::Value>> {
    let tools = tools.map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;
    spawn_admin_audit(
        state.pool.clone(),
        audit_event,
        name.to_string(),
        format!("MCP server {}", action),
        None,
        None,
        None,
    );
    info!(name = %name, "MCP server {}", action);
    let mut resp = serde_json::json!({ "name": name });
    if let Some(t) = tools {
        resp["tools"] = serde_json::json!(t);
    }
    ok_data(resp)
}

/// POST /api/mcp/servers/:name/restart
pub async fn restart_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let result = state.mcp_manager.restart_server(&name).await.map(Some);
    server_lifecycle(&state, &name, "restarted", "MCP_SERVER_RESTARTED", result).await
}

/// POST /api/mcp/servers/:name/start
pub async fn start_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let result = state.mcp_manager.start_server(&name).await.map(Some);
    server_lifecycle(&state, &name, "started", "MCP_SERVER_STARTED", result).await
}

/// POST /api/mcp/servers/:name/stop
pub async fn stop_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let result = state.mcp_manager.stop_server(&name).await.map(|()| None);
    server_lifecycle(&state, &name, "stopped", "MCP_SERVER_STOPPED", result).await
}

// ============================================================
// Direct Tool Call API (MGP §5.6 Coordinator Pattern)
// ============================================================

#[derive(Deserialize)]
pub struct CallMcpToolRequest {
    /// Which server provides the tool. Optional: omit it and the kernel reads
    /// the answer out of its own tool index.
    ///
    /// A coordinator that already holds a server id keeps passing it. A caller
    /// that only knows a tool name — a bridge forwarding for a harness, which
    /// was handed schemas and never a topology — should not have to carry a
    /// mapping the kernel owns and can change under it.
    #[serde(default)]
    pub server_id: String,
    pub tool_name: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

/// POST /api/mcp/call — Direct tool call for coordinator-pattern servers (MGP §5.6, §19.1).
/// Delegation validation (anti-spoofing, permission intersection, chain depth) is enforced
/// by `call_server_tool()` when `_mgp.delegation` is present in arguments.
pub async fn call_mcp_tool(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CallMcpToolRequest>,
) -> AppResult<Json<serde_json::Value>> {
    // Which caller this runs as is decided by the credential presented, not by
    // anything in the body — see `resolve_tool_caller`. An agent token names one
    // agent and keeps the per-agent capability gate; the admin key is the
    // coordinator credential and still runs as System.
    let caller = crate::handlers::resolve_tool_caller(&state, &headers).await?;

    // Kernel-native tools have no server to route to, and the index below only
    // knows about connected MCP servers — so they have to be dispatched before
    // routing is attempted, not after it fails.
    //
    // This is the path an external harness takes: a CLI agent runs as a
    // subprocess and calls back in over HTTP, so *every* tool it was offered
    // arrives here. `collect_tool_schemas_for_agent` offers the `mgp.tools.*`
    // discovery pair, `mgp.skill.load` and the `mgp.operator.*` pair to agents
    // that hold no grants at all, and before this branch existed not one of them
    // could be called through this endpoint — the kernel answered "no connected
    // MCP server provides tool", naming a tool it had just advertised itself.
    // The in-process agentic loop never hit it because it calls `execute_tool`
    // directly.
    //
    // `execute_tool` applies the same capability gate, so routing past the index
    // does not route past the gate: kernel RBAC still refuses an explicit Deny.
    if McpClientManager::is_kernel_native_tool(&body.tool_name) {
        let result = state
            .mcp_manager
            .execute_tool(&caller, &body.tool_name, body.arguments)
            .await
            .map_err(|failure| match failure {
                // A rejection is a policy answer, not a fault: it carries the
                // reason the caller is meant to read, so it travels as the
                // refusal text rather than being flattened into a 500.
                cloto_shared::ToolFailure::Rejection(r) => AppError::Validation(r.reason),
                // Kernel tools refuse a call the caller got wrong — a missing
                // argument, the wrong kind of caller — with an MGP-typed error,
                // whose message is written for the caller and goes out with its
                // code. Anything untyped is a fault, and its text stays in the
                // log: widening this to forward it would put the contents of
                // genuine faults into response bodies.
                cloto_shared::ToolFailure::Error(e) => {
                    match e.downcast::<crate::managers::mcp_mgp::MgpError>() {
                        Ok(mgp) => AppError::Mgp(Box::new(mgp)),
                        Err(other) => AppError::Internal(other),
                    }
                }
            })?;
        return ok_data(result);
    }

    // Resolve the provider when the caller did not name one. The index is the
    // kernel's own answer to "who serves this tool" for everything a server
    // provides.
    let server_id = if body.server_id.is_empty() {
        state
            .mcp_manager
            .get_tool_server_id(&body.tool_name)
            .await
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "No connected MCP server provides tool '{}'",
                    body.tool_name
                ))
            })?
    } else {
        body.server_id.clone()
    };

    // Pre-flight: reject immediately if server is known-dead (bug-354)
    if !state.mcp_manager.is_server_alive(&server_id).await {
        return Err(AppError::Validation(format!(
            "MCP server '{server_id}' is not connected"
        )));
    }

    // Under the admin key this is System, and per-agent scoping flows only
    // through an `_mgp.delegation` envelope (original_actor) handled inside
    // resolve_tool_call_target, never a body agent_id. Under an agent token the
    // caller is that agent and `enforce_caller_grant` applies directly.
    let result = state
        .mcp_manager
        .call_server_tool(&caller, &server_id, &body.tool_name, body.arguments)
        .await
        .map_err(
            |e| match e.downcast::<crate::managers::mcp_mgp::MgpError>() {
                Ok(mgp) => AppError::Mgp(Box::new(mgp)),
                Err(other) => AppError::Internal(other),
            },
        )?;

    let value = serde_json::to_value(result)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to serialize result: {}", e)))?;
    ok_data(value)
}

// ============================================================
// YOLO Mode API
// ============================================================

/// GET /api/settings/yolo
pub async fn get_yolo_mode(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let enabled = state
        .mcp_manager
        .yolo_mode
        .load(std::sync::atomic::Ordering::Relaxed);
    ok_data(serde_json::json!({ "enabled": enabled }))
}

/// PUT /api/settings/yolo
pub async fn set_yolo_mode(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let enabled = body
        .get("enabled")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    state
        .mcp_manager
        .yolo_mode
        .store(enabled, std::sync::atomic::Ordering::Relaxed);

    // Persist to DB so the setting survives kernel restarts
    let value = if enabled { "true" } else { "false" };
    if let Err(e) = sqlx::query(
        "INSERT OR REPLACE INTO plugin_configs (plugin_id, config_key, config_value) VALUES ('kernel', 'yolo_mode', ?)"
    )
        .bind(value)
        .execute(&state.pool)
        .await
    {
        tracing::error!(error = %e, "Failed to persist YOLO mode to DB");
    }

    if enabled {
        tracing::warn!("⚠️  YOLO mode enabled — all MCP permissions auto-approved. This bypasses security isolation.");
    } else {
        tracing::info!("YOLO mode disabled via API");
        // bug-438: purge every access grant an agent self-issued via
        // mgp.access.grant during the YOLO window, so a temporary trust window
        // cannot durably widen the admin-curated allow-list once YOLO is off.
        // These rows carry a `yolo-grant:` prefix in `granted_by` (see
        // execute_access_grant); admin grants (put_agent_mcp_access) use a
        // different granted_by and are preserved.
        match sqlx::query("DELETE FROM mcp_access_control WHERE granted_by LIKE 'yolo-grant:%'")
            .execute(&state.pool)
            .await
        {
            Ok(res) if res.rows_affected() > 0 => tracing::warn!(
                purged = res.rows_affected(),
                "Purged YOLO-mode self-grants on YOLO disable (bug-438)"
            ),
            Ok(_) => {}
            Err(e) => tracing::error!(error = %e, "Failed to purge YOLO self-grants (bug-438)"),
        }
    }

    spawn_admin_audit(
        state.pool.clone(),
        "YOLO_MODE_CHANGED",
        "system".to_string(),
        format!("YOLO mode set to {}", enabled),
        None,
        None,
        None,
    );

    ok_data(serde_json::json!({ "enabled": enabled }))
}

// ============================================================
// Response Language Injection API
// ============================================================
// Controls whether the kernel injects `metadata["response_language"]` into
// every agent dict sent to MCP think tools, so the system prompt asks the
// LLM to reply in the operator's language. Default ON. Agents with explicit
// language guidance in their description still take precedence.

/// GET /api/settings/language
pub async fn get_response_language(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let enabled = state
        .mcp_manager
        .inject_response_language
        .load(std::sync::atomic::Ordering::Relaxed);
    let language = state.mcp_manager.response_language.read().await.clone();
    ok_data(serde_json::json!({
        "enabled": enabled,
        "language": language,
    }))
}

/// PUT /api/settings/language
pub async fn set_response_language(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    // Both fields optional — caller may update toggle without language or vice versa.
    let enabled_opt = body.get("enabled").and_then(serde_json::Value::as_bool);
    let language_opt = body
        .get("language")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    if let Some(enabled) = enabled_opt {
        state
            .mcp_manager
            .inject_response_language
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
        let value = if enabled { "true" } else { "false" };
        if let Err(e) = sqlx::query(
            "INSERT OR REPLACE INTO plugin_configs (plugin_id, config_key, config_value) VALUES ('kernel', 'inject_response_language', ?)"
        )
            .bind(value)
            .execute(&state.pool)
            .await
        {
            tracing::error!(error = %e, "Failed to persist inject_response_language to DB");
        }
    }

    if let Some(language) = language_opt.clone() {
        *state.mcp_manager.response_language.write().await = language.clone();
        if let Err(e) = sqlx::query(
            "INSERT OR REPLACE INTO plugin_configs (plugin_id, config_key, config_value) VALUES ('kernel', 'response_language', ?)"
        )
            .bind(&language)
            .execute(&state.pool)
            .await
        {
            tracing::error!(error = %e, "Failed to persist response_language to DB");
        }
    }

    let enabled = state
        .mcp_manager
        .inject_response_language
        .load(std::sync::atomic::Ordering::Relaxed);
    let language = state.mcp_manager.response_language.read().await.clone();
    tracing::info!(
        enabled = enabled,
        language = %language,
        "Response language settings updated"
    );

    ok_data(serde_json::json!({
        "enabled": enabled,
        "language": language,
    }))
}

// ============================================================
// CRON Recursion Limit API
// ============================================================

/// GET /api/settings/max-cron-generation
pub async fn get_max_cron_generation(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let val = state
        .max_cron_generation
        .load(std::sync::atomic::Ordering::Relaxed);
    ok_data(serde_json::json!({ "value": val, "max": 6 }))
}

/// PUT /api/settings/max-cron-generation
pub async fn set_max_cron_generation(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let raw = body
        .get("value")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(2);
    if raw > 6 {
        return Err(AppError::Validation(
            "max_cron_generation must be 0-6".into(),
        ));
    }
    let val = raw as u8;
    state
        .max_cron_generation
        .store(val, std::sync::atomic::Ordering::Relaxed);

    tracing::info!("max_cron_generation set to {} via API", val);

    spawn_admin_audit(
        state.pool.clone(),
        "MAX_CRON_GENERATION_CHANGED",
        "system".to_string(),
        format!("max_cron_generation set to {}", val),
        None,
        None,
        None,
    );

    ok_data(serde_json::json!({ "value": val }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_validation(result: AppResult<()>) -> bool {
        matches!(result, Err(AppError::Validation(_)))
    }

    #[test]
    fn server_name_accepts_dotted_ids() {
        // Mirror the built-in servers and the Add Server modal hint.
        for name in [
            "tool.terminal",
            "memory.cpersona",
            "mind.local",
            "x-browser",
            "github-bridge",
            "a",
            "foo.bar.baz",
            "with_underscores",
        ] {
            assert!(
                validate_server_name(name).is_ok(),
                "expected '{}' to be accepted",
                name
            );
        }
    }

    #[test]
    fn remote_server_request_keeps_connection_fields() {
        let body = serde_json::json!({
            "transport": "streamable-http",
            "url": "https://memory.example.com/mcp",
            "auth_token": "test-bearer",
            "auto_restart": true,
            "display_name": "Remote memory"
        });
        let Ok(Some(config)) = remote_server_config("memory.example", &body) else {
            panic!("valid remote request must produce a complete config")
        };

        assert_eq!(config.id, "memory.example");
        assert_eq!(config.transport, "streamable-http");
        assert_eq!(
            config.url.as_deref(),
            Some("https://memory.example.com/mcp")
        );
        assert_eq!(config.auth_token.as_deref(), Some("test-bearer"));
        assert_eq!(config.auto_restart, Some(true));
    }

    #[test]
    fn remote_server_request_rejects_stdio_fields_and_missing_url() {
        for body in [
            serde_json::json!({"transport": "streamable-http"}),
            serde_json::json!({
                "transport": "streamable-http",
                "url": "https://memory.example.com/mcp",
                "command": "python"
            }),
            serde_json::json!({"transport": "websocket", "url": "wss://example.com"}),
        ] {
            assert!(
                remote_server_config("memory.example", &body).is_err(),
                "invalid remote request must be rejected: {body}"
            );
        }
        assert!(matches!(
            remote_server_config("local", &serde_json::json!({})),
            Ok(None)
        ));
    }

    #[tokio::test]
    async fn remote_server_settings_never_return_the_bearer_token() {
        let state = crate::test_utils::create_test_app_state(Some("admin-key".into())).await;
        crate::db::save_mcp_server(
            &state.pool,
            &crate::db::McpServerRecord {
                name: "memory.example".to_string(),
                transport: "streamable-http".to_string(),
                url: Some("https://memory.example.com/mcp".to_string()),
                auth_token: Some("test-bearer".to_string()),
                is_active: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("X-API-Key", "admin-key".parse().unwrap());

        let Ok(Json(response)) =
            get_mcp_server_settings(State(state), Path("memory.example".to_string()), headers)
                .await
        else {
            panic!("settings request must succeed")
        };

        assert_eq!(
            response.pointer("/data/auth_token_configured"),
            Some(&serde_json::json!(true))
        );
        assert_eq!(
            response.pointer("/data/url"),
            Some(&serde_json::json!("https://memory.example.com/mcp"))
        );
        assert!(
            !response.to_string().contains("test-bearer"),
            "settings response must not expose the stored bearer token"
        );
    }

    #[test]
    fn server_name_rejects_traversal_shapes() {
        for name in [".hidden", "trailing.", "foo..bar", "..", "."] {
            assert!(
                is_validation(validate_server_name(name)),
                "expected '{}' to be rejected",
                name
            );
        }
    }

    #[test]
    fn server_name_rejects_non_ascii_and_special_chars() {
        for name in ["hello world", "foo/bar", "foo\\bar", "サーバー", ""] {
            assert!(
                is_validation(validate_server_name(name)),
                "expected '{}' to be rejected",
                name
            );
        }
    }

    #[test]
    fn server_name_length_limits() {
        assert!(is_validation(validate_server_name("")));
        assert!(validate_server_name(&"a".repeat(64)).is_ok());
        assert!(is_validation(validate_server_name(&"a".repeat(65))));
    }

    // ── provider resolution on /api/mcp/call (Task #1306) ──
    //
    // These drive the handler, not a helper, because the question is whether
    // the endpoint consults the index — a caller that never learned a topology
    // has to be able to omit `server_id` and still reach the right server.
    //
    // The two outcomes are told apart by which stage refused: routing speaks
    // before the liveness pre-flight, so "no connected MCP server provides"
    // means the index found nothing, and "is not connected" means it resolved
    // and named what it resolved to. Same distinction `agent_token_caller_test`
    // relies on one stage further in.

    async fn call_with(
        state: &Arc<crate::AppState>,
        token: &str,
        server_id: &str,
        tool_name: &str,
    ) -> String {
        let mut headers = HeaderMap::new();
        headers.insert(
            crate::managers::agent_token::AGENT_TOKEN_HEADER,
            token.parse().unwrap(),
        );
        let body = CallMcpToolRequest {
            server_id: server_id.to_string(),
            tool_name: tool_name.to_string(),
            arguments: serde_json::json!({}),
        };
        match call_mcp_tool(State(state.clone()), headers, Json(body)).await {
            Ok(_) => panic!("a handle with no client cannot answer a call"),
            Err(AppError::Validation(msg)) => msg,
            // Anything else means the call was refused somewhere other than the
            // two stages under test — a green assertion here would be measuring
            // the wrong refusal.
            Err(AppError::Cloto(e)) => panic!("refused before routing: {e}"),
            Err(AppError::NotFound(m) | AppError::Conflict(m)) => {
                panic!("refused before routing: {m}")
            }
            Err(AppError::Internal(e)) => panic!("refused before routing: {e}"),
            Err(AppError::Mgp(e)) => panic!("refused before routing: {e}"),
        }
    }

    #[tokio::test]
    async fn an_omitted_server_id_is_resolved_from_the_tool_index() {
        let state = crate::test_utils::create_test_app_state(Some("admin-key".into())).await;
        state
            .mcp_manager
            .insert_test_server_providing("srv.memory", "recall")
            .await;
        let token = state.agent_tokens.mint_default("agent.growth").await;

        let err = call_with(&state, &token, "", "recall").await;

        assert!(
            err.contains("srv.memory") && err.contains("not connected"),
            "an omitted server_id must resolve to the indexed provider and get \
             past routing; got: {err}"
        );
    }

    /// The path an external harness actually takes.
    ///
    /// A CLI agent runs as a subprocess and calls back in over HTTP, so every
    /// tool it was offered arrives at this handler — including the kernel's own,
    /// which no server provides and the index below therefore cannot resolve.
    /// Before this was handled, the kernel refused tools it had itself
    /// advertised, and nothing caught it: the in-process loop calls
    /// `execute_tool` directly and never comes through here, so a test written
    /// against `execute_tool` passes while the real caller gets a 400.
    #[tokio::test]
    async fn a_kernel_native_tool_is_dispatched_here_rather_than_routed_to_a_server() {
        let state = crate::test_utils::create_test_app_state(Some("admin-key".into())).await;
        let token = state.agent_tokens.mint_default("agent.growth").await;

        let mut headers = HeaderMap::new();
        headers.insert(
            crate::managers::agent_token::AGENT_TOKEN_HEADER,
            token.parse().unwrap(),
        );
        let out = call_mcp_tool(
            State(state.clone()),
            headers,
            Json(CallMcpToolRequest {
                server_id: String::new(),
                tool_name: "mgp.operator.ask".to_string(),
                arguments: serde_json::json!({ "title": "may I?" }),
            }),
        )
        .await
        .unwrap_or_else(|_| {
            panic!(
                "a tool the kernel offers must be callable through the endpoint it \
                 offers it on"
            )
        });

        let item_id = out.0["data"]["item_id"]
            .as_str()
            .expect("the caller gets the id of the question it raised");
        let item = crate::db::get_notification(&state.pool, item_id)
            .await
            .unwrap()
            .expect("the question reached the store");
        assert_eq!(item.kind, crate::db::NotificationKind::Proposal);
        // The caller was an agent token, so the question is attributed to that
        // agent and not to the coordinator.
        assert_eq!(item.agent_id.as_deref(), Some("agent.growth"));
    }

    #[tokio::test]
    async fn an_omitted_server_id_for_an_unindexed_tool_is_refused_by_name() {
        let state = crate::test_utils::create_test_app_state(Some("admin-key".into())).await;
        state
            .mcp_manager
            .insert_test_server_providing("srv.memory", "recall")
            .await;
        let token = state.agent_tokens.mint_default("agent.growth").await;

        let err = call_with(&state, &token, "", "no_such_tool").await;

        assert!(
            err.contains("No connected MCP server provides tool 'no_such_tool'"),
            "a tool nothing provides must be refused at routing, not guessed \
             at; got: {err}"
        );
    }

    #[tokio::test]
    async fn a_caller_that_names_a_server_is_still_taken_at_its_word() {
        let state = crate::test_utils::create_test_app_state(Some("admin-key".into())).await;
        state
            .mcp_manager
            .insert_test_server_providing("srv.memory", "recall")
            .await;
        let token = state.agent_tokens.mint_default("agent.growth").await;

        // The index would have said srv.memory. An explicit id must win, or a
        // coordinator could be silently rerouted to a server it did not pick.
        let err = call_with(&state, &token, "srv.elsewhere", "recall").await;

        assert!(
            err.contains("srv.elsewhere"),
            "an explicit server_id must not be overridden by the index; got: {err}"
        );
    }

    // ── what a refusal says when it reaches a harness ──
    //
    // The caller on this endpoint is often a model running as a subprocess. When
    // it gets an argument wrong, the body is the only thing it reads before
    // trying again. These assert on the rendered response, not on the
    // `AppError` variant, because the withholding happens in `into_response`:
    // an `Internal` carries the full reason right up to the moment it is
    // replaced with a generic line.

    async fn call_and_render(
        state: &Arc<crate::AppState>,
        headers: HeaderMap,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> (axum::http::StatusCode, String) {
        let (status, body) = call_and_render_body(state, headers, tool_name, arguments).await;
        let message = body["error"]["message"]
            .as_str()
            .unwrap_or_else(|| panic!("the refusal carries a message field; got {body}"))
            .to_string();
        (status, message)
    }

    async fn call_and_render_body(
        state: &Arc<crate::AppState>,
        headers: HeaderMap,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> (axum::http::StatusCode, serde_json::Value) {
        use axum::response::IntoResponse;

        let response = match call_mcp_tool(
            State(state.clone()),
            headers,
            Json(CallMcpToolRequest {
                server_id: String::new(),
                tool_name: tool_name.to_string(),
                arguments,
            }),
        )
        .await
        {
            Ok(_) => panic!("{tool_name} was expected to refuse this call"),
            Err(e) => e.into_response(),
        };
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        (status, body)
    }

    async fn agent_headers(state: &Arc<crate::AppState>) -> HeaderMap {
        let token = state.agent_tokens.mint_default("agent.growth").await;
        let mut headers = HeaderMap::new();
        headers.insert(
            crate::managers::agent_token::AGENT_TOKEN_HEADER,
            token.parse().unwrap(),
        );
        headers
    }

    #[tokio::test]
    async fn a_missing_argument_is_named_to_the_harness_that_left_it_out() {
        let state = crate::test_utils::create_test_app_state(Some("admin-key".into())).await;

        // Both refusals seen on the deployed kernel, each from a different
        // module, so one wired helper cannot stand in for the other.
        for (tool, missing) in [
            ("mgp.operator.ask", "title"),
            ("mgp.tools.discover", "query"),
        ] {
            let (status, message) = call_and_render(
                &state,
                agent_headers(&state).await,
                tool,
                serde_json::json!({}),
            )
            .await;
            assert_eq!(
                status,
                axum::http::StatusCode::BAD_REQUEST,
                "{tool}: a call the caller got wrong is not a server fault; got: {message}"
            );
            assert!(
                message.contains(&format!("Missing required parameter: {missing}")),
                "{tool}: the caller must be told which argument to add; got: {message}"
            );
        }
    }

    /// One row per place a kernel tool refuses a value it was given. Each is its
    /// own call site, so each gets its own row: a site left untyped would answer
    /// this caller with a 500 while every other row stays green.
    #[tokio::test]
    async fn each_unusable_value_is_named_to_the_caller() {
        let state = crate::test_utils::create_test_app_state(Some("admin-key".into())).await;
        // Two of these tools are privileged. Without the flag they refuse before
        // they ever look at the value, and the row would be measuring that.
        state
            .mcp_manager
            .yolo_mode
            .store(true, std::sync::atomic::Ordering::Relaxed);

        let grant = |entry_type: &str, permission: &str| {
            serde_json::json!({
                "agent_id": "agent.growth",
                "server_id": "srv.memory",
                "entry_type": entry_type,
                "permission": permission,
            })
        };
        let rows = [
            (
                "mgp.kernel.create_mcp_server",
                serde_json::json!({ "name": "srv", "code": "", "server_type": "hybrid" }),
                "Invalid server_type 'hybrid'",
            ),
            (
                "mgp.kernel.create_mcp_server",
                serde_json::json!({ "name": "", "code": "" }),
                "Server name must be 1-64 characters",
            ),
            (
                "mgp.kernel.create_mcp_server",
                serde_json::json!({ "name": "no spaces", "code": "" }),
                "Server name must contain only alphanumeric",
            ),
            (
                "mgp.access.grant",
                grant("sideways", "allow"),
                "Invalid entry_type: 'sideways'",
            ),
            (
                "mgp.access.grant",
                grant("server_grant", "maybe"),
                "Invalid permission: 'maybe'",
            ),
            (
                "mgp.tools.discover",
                serde_json::json!({ "query": "memory", "strategy": "category" }),
                "Category search requires filter.categories",
            ),
        ];
        for (tool, arguments, says) in rows {
            let (status, message) =
                call_and_render(&state, agent_headers(&state).await, tool, arguments).await;
            assert_eq!(
                status,
                axum::http::StatusCode::BAD_REQUEST,
                "{tool} ({says}): got: {message}"
            );
            assert!(
                message.contains(says),
                "{tool}: the caller must be told what is wrong with the value; got: {message}"
            );
        }
    }

    #[tokio::test]
    async fn an_unusable_value_and_a_missing_grant_are_said_in_words_too() {
        let state = crate::test_utils::create_test_app_state(Some("admin-key".into())).await;

        // Present but unusable: a different helper from the missing-argument one.
        let (status, message) = call_and_render(
            &state,
            agent_headers(&state).await,
            "mgp.events.subscribe",
            serde_json::json!({ "channels": [] }),
        )
        .await;
        assert_eq!(
            status,
            axum::http::StatusCode::BAD_REQUEST,
            "got: {message}"
        );
        assert!(
            message.contains("channels must not be empty"),
            "the caller must be told what is wrong with the value; got: {message}"
        );

        // Well-formed, but the agent holds no grant on the server it named. That
        // is a refusal to stop asking, and a 500 reads as one to retry.
        let (status, message) = call_and_render(
            &state,
            agent_headers(&state).await,
            "mgp.events.subscribe",
            serde_json::json!({
                "server_id": "srv.memory",
                "agent_id": "agent.growth",
                "channels": ["notifications/progress"],
            }),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::FORBIDDEN, "got: {message}");
        assert!(
            message.contains("has no grant for server 'srv.memory'"),
            "the caller must be told the refusal is about access; got: {message}"
        );
    }

    /// Registers `server_id` and grants it to the test agent. The callback and
    /// lifecycle tools authorize against the named server before they look
    /// anything up, so without a grant every row would measure that refusal.
    async fn grant_test_agent(state: &Arc<crate::AppState>, server_id: &str) {
        sqlx::query(
            "INSERT OR IGNORE INTO mcp_servers (name, command, created_at, default_policy) \
             VALUES (?, 'noop', 0, 'opt-in')",
        )
        .bind(server_id)
        .execute(&state.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO mcp_access_control \
             (entry_type, agent_id, server_id, tool_name, permission, granted_at) \
             VALUES ('server_grant', 'agent.growth', ?, NULL, 'allow', 't0')",
        )
        .bind(server_id)
        .execute(&state.pool)
        .await
        .unwrap();
    }

    /// One row per place a kernel tool is asked about something that is not
    /// there. A missing tool is also a 404, so the status cannot tell these rows
    /// from that case — each row reads the MGP code in the body as well.
    #[tokio::test]
    async fn each_missing_resource_is_named_to_the_caller() {
        let state = crate::test_utils::create_test_app_state(Some("admin-key".into())).await;
        // `mgp.discovery.deregister` is privileged and refuses before the lookup
        // without this.
        state
            .mcp_manager
            .yolo_mode
            .store(true, std::sync::atomic::Ordering::Relaxed);

        // Granted, but never loaded by the kernel.
        grant_test_agent(&state, "srv.gone").await;
        assert!(state
            .mcp_manager
            .register_test_callback("cb-gone", "srv.gone"));
        // Loaded, but with no live client behind it.
        state
            .mcp_manager
            .insert_test_server_providing("srv.memory", "recall")
            .await;
        grant_test_agent(&state, "srv.memory").await;
        assert!(state
            .mcp_manager
            .register_test_callback("cb-idle", "srv.memory"));

        let answer = |id: &str| serde_json::json!({ "callback_id": id, "response": "yes", "agent_id": "agent.growth" });
        let rows = [
            (
                "mgp.events.replay",
                serde_json::json!({ "subscription_id": "sub-nope" }),
                404,
                4004,
                "Subscription 'sub-nope' not found",
            ),
            (
                "mgp.callback.respond",
                answer("cb-nope"),
                404,
                4004,
                "Callback 'cb-nope' not found",
            ),
            (
                "mgp.callback.respond",
                answer("cb-gone"),
                404,
                4004,
                "Server 'srv.gone' not found",
            ),
            // An answer that reached no server did not use the callback up, so
            // the second attempt meets the same missing server, not "already
            // answered".
            (
                "mgp.callback.respond",
                answer("cb-gone"),
                404,
                4004,
                "Server 'srv.gone' not found",
            ),
            (
                "mgp.callback.respond",
                answer("cb-idle"),
                503,
                2000,
                "Server 'srv.memory' not connected",
            ),
            (
                "mgp.callback.respond",
                answer("cb-idle"),
                503,
                2000,
                "Server 'srv.memory' not connected",
            ),
            (
                "mgp.lifecycle.shutdown",
                serde_json::json!({
                    "server_id": "srv.gone",
                    "agent_id": "agent.growth",
                    "reason": "probe",
                }),
                404,
                4004,
                "Server 'srv.gone' not found",
            ),
            (
                "mgp.discovery.deregister",
                serde_json::json!({ "id": "srv.nope" }),
                404,
                4004,
                "Server 'srv.nope' not found",
            ),
        ];
        for (tool, arguments, status, code, says) in rows {
            let (got, body) =
                call_and_render_body(&state, agent_headers(&state).await, tool, arguments).await;
            assert_eq!(got.as_u16(), status, "{tool} ({says}): got {body}");
            assert_eq!(body["error"]["code"], code, "{tool} ({says}): got {body}");
            let message = body["error"]["message"].as_str().unwrap_or_default();
            assert!(
                message.contains(says),
                "{tool}: the caller must be told what was not there; got: {message}"
            );
            let retryable = &body["error"]["data"]["_mgp"]["retryable"];
            if code == 4004 {
                // Sending the same id again cannot start resolving.
                assert_eq!(retryable, false, "{tool} ({says}): got {body}");
            } else {
                // The answer can be sent again, but a server that went away may
                // no longer know the callback, so no hint may promise delivery.
                assert!(retryable.is_null(), "{tool} ({says}): got {body}");
            }
        }
    }

    /// A server id that is already taken is not a missing resource. Answering it
    /// with the 404 of the rows above would send the caller off to check an id
    /// that is plainly there.
    #[tokio::test]
    async fn registering_a_taken_server_id_is_a_conflict() {
        let state = crate::test_utils::create_test_app_state(Some("admin-key".into())).await;
        // `mgp.discovery.register` is privileged and refuses before the
        // duplicate check without this.
        state
            .mcp_manager
            .yolo_mode
            .store(true, std::sync::atomic::Ordering::Relaxed);
        state
            .mcp_manager
            .insert_test_server_providing("srv.memory", "recall")
            .await;

        let (status, body) = call_and_render_body(
            &state,
            agent_headers(&state).await,
            "mgp.discovery.register",
            serde_json::json!({ "id": "srv.memory", "command": "noop", "transport": "stdio" }),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::CONFLICT, "got {body}");
        assert_eq!(body["error"]["code"], 4101, "got {body}");
        let message = body["error"]["message"].as_str().unwrap_or_default();
        assert!(
            message.contains("Server 'srv.memory' is already registered"),
            "the caller must be told which id is taken; got: {message}"
        );
    }

    #[tokio::test]
    async fn a_caller_of_the_wrong_kind_is_told_so_rather_than_told_nothing() {
        let state = crate::test_utils::create_test_app_state(Some("admin-key".into())).await;

        // The coordinator key runs as System, and a question needs an agent to
        // belong to. The arguments are valid, so the refusal is about who asked.
        for (tool, arguments, says) in [
            (
                "mgp.operator.ask",
                serde_json::json!({ "title": "may I?" }),
                "identifies the asker by agent",
            ),
            (
                "mgp.operator.replies",
                serde_json::json!({}),
                "returns one agent's questions",
            ),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(
                crate::handlers::ADMIN_API_KEY_HEADER,
                "admin-key".parse().unwrap(),
            );
            let (status, message) = call_and_render(&state, headers, tool, arguments).await;

            assert_eq!(
                status,
                axum::http::StatusCode::FORBIDDEN,
                "{tool}: got: {message}"
            );
            assert!(
                message.contains(says),
                "{tool}: the refusal must say what kind of caller the tool needs; got: {message}"
            );
        }
    }

    /// The call sites the tests above reach are a few of the dozens changed
    /// together. A new kernel tool written in the old shape would compile, pass
    /// every other test, and answer a harness with a 500 again — so the old shape
    /// is refused here. Lexical by nature: it catches the idiom being copied, not
    /// a new wording of the same mistake.
    #[test]
    fn no_kernel_tool_reports_a_missing_argument_as_an_untyped_error() {
        let sources = [
            (
                "mcp_kernel_tool.rs",
                include_str!("../managers/mcp_kernel_tool.rs"),
            ),
            (
                "mcp_discovery.rs",
                include_str!("../managers/mcp_discovery.rs"),
            ),
            (
                "mcp_tool_discovery.rs",
                include_str!("../managers/mcp_tool_discovery.rs"),
            ),
            ("mcp_events.rs", include_str!("../managers/mcp_events.rs")),
        ];
        let offenders: Vec<String> = sources
            .iter()
            .flat_map(|(file, src)| {
                src.lines()
                    .enumerate()
                    .filter(|(_, line)| line.contains("anyhow!(\"Missing required parameter"))
                    .map(move |(i, line)| format!("{file}:{}: {}", i + 1, line.trim()))
            })
            .collect();
        assert!(
            offenders.is_empty(),
            "use mcp_mgp::missing_tool_arg, which /api/mcp/call forwards; an untyped \
             error is withheld as an internal fault:\n{}",
            offenders.join("\n")
        );
    }

    /// The control for the tests above. Without it, answering every failure
    /// with its own text would pass them too — and would hand a client the
    /// contents of genuine internal faults, which `AppError::Internal` exists to
    /// keep in the log.
    #[tokio::test]
    async fn a_genuine_internal_fault_still_keeps_its_contents_to_the_log() {
        let state = crate::test_utils::create_test_app_state(Some("admin-key".into())).await;
        let headers = agent_headers(&state).await;

        // A closed pool fails the capability lookup that runs before the tool:
        // a real runtime fault, on the same endpoint, with a reason worth hiding.
        state.pool.close().await;

        let (status, message) = call_and_render(
            &state,
            headers,
            "mgp.operator.ask",
            serde_json::json!({ "title": "may I?" }),
        )
        .await;

        assert_eq!(
            status,
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "got: {message}"
        );
        assert_eq!(message, "An internal error occurred");
    }
}
