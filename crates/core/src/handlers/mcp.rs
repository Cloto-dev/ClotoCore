use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use serde::Deserialize;
use std::{collections::HashMap, sync::Arc};
use tracing::{error, info};

use crate::{AppError, AppResult, AppState};

use super::{check_auth, spawn_admin_audit};

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
pub async fn get_plugins(State(state): State<Arc<AppState>>) -> AppResult<Json<serde_json::Value>> {
    let manifests = state
        .plugin_manager
        .list_plugins_with_settings(&state.registry)
        .await?;
    Ok(Json(serde_json::json!(manifests)))
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
    Ok(Json(serde_json::json!(config)))
}

/// Update a single plugin configuration key-value pair.
///
/// **Route:** `PUT /api/plugins/:id/config`
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
        let envelope = crate::EnvelopedEvent::system(cloto_shared::ClotoEventData::ConfigUpdated {
            plugin_id: id.clone(),
            config: full_config,
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

    Ok(Json(serde_json::json!({ "status": "success" })))
}

/// Batch apply plugin enabled/disabled settings.
///
/// **Route:** `POST /api/plugins/settings`
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
) -> AppResult<Json<bool>> {
    check_auth(&state, &headers)?;
    info!(
        count = payload.len(),
        "📥 Received plugin settings apply request"
    );

    let settings = payload.into_iter().map(|i| (i.id, i.is_active)).collect();

    state.plugin_manager.apply_settings(settings).await?;
    Ok(Json(true))
}

/// Grant a permission to a plugin.
///
/// **Route:** `POST /api/plugins/:id/permissions`
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

    // イベントループに通知して Capability を注入させる
    let envelope = crate::EnvelopedEvent::system(cloto_shared::ClotoEventData::PermissionGranted {
        plugin_id: id.clone(),
        permission: payload.permission.clone(),
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

    Ok(Json(serde_json::json!({ "status": "success" })))
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
    Ok(Json(
        serde_json::json!({ "plugin_id": id, "permissions": list }),
    ))
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

    Ok(Json(serde_json::json!({ "status": "success" })))
}

// ============================================================
// MCP Dynamic Server Management
// ============================================================

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

    // Name validation
    if name.is_empty() || name.len() > 64 {
        return Err(AppError::Validation(
            "Server name must be 1-64 characters".into(),
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(AppError::Validation(
            "Server name must contain only alphanumeric characters, underscores, and hyphens"
                .into(),
        ));
    }

    // Determine command/args: either explicit or auto-generated from code
    let (command, args, script_content) =
        if let Some(code) = body.get("code").and_then(|v| v.as_str()) {
            let description = body
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("A dynamically generated MCP server.");

            // Auto-generate MCP server Python script
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

            // Write script to scripts/ directory
            let script_filename = format!("mcp_{}.py", name);
            let scripts_dir = std::path::Path::new("scripts");
            if !scripts_dir.exists() {
                std::fs::create_dir_all(scripts_dir).map_err(|e| {
                    AppError::Internal(anyhow::anyhow!("Failed to create scripts directory: {}", e))
                })?;
            }
            std::fs::write(scripts_dir.join(&script_filename), &script).map_err(|e| {
                AppError::Internal(anyhow::anyhow!("Failed to write MCP server script: {}", e))
            })?;

            let python = if cfg!(windows) { "python" } else { "python3" };
            (
                python.to_string(),
                vec![format!("scripts/{}", script_filename)],
                Some(script),
            )
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

    // Add server via McpClientManager (handles connection + DB persistence)
    let tool_names = state
        .mcp_manager
        .add_dynamic_server(
            name.to_string(),
            command.clone(),
            args.clone(),
            script_content,
            body.get("description")
                .and_then(|v| v.as_str())
                .map(String::from),
        )
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to add MCP server: {}", e)))?;

    tracing::info!(name = %name, tools = ?tool_names, "🔌 Dynamic MCP server added");

    Ok(Json(serde_json::json!({
        "status": "created",
        "name": name,
        "tools": tool_names,
    })))
}

pub async fn list_mcp_servers(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let servers = state.mcp_manager.list_servers().await;

    Ok(Json(serde_json::json!({
        "servers": servers,
        "count": servers.len(),
    })))
}

pub async fn delete_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    // Remove from McpClientManager (handles disconnect + DB deactivation)
    // Config-loaded servers cannot be deleted — return 400 with guidance
    state
        .mcp_manager
        .remove_dynamic_server(&name)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("config-loaded") {
                AppError::Validation(msg)
            } else {
                AppError::Internal(anyhow::anyhow!("{}", e))
            }
        })?;

    // Remove auto-generated script file if it exists
    let script_path = std::path::Path::new("scripts").join(format!("mcp_{}.py", name));
    if script_path.exists() {
        let _ = std::fs::remove_file(&script_path);
    }

    tracing::info!(name = %name, "🗑️ MCP server removed");

    Ok(Json(serde_json::json!({
        "status": "deleted",
        "name": name,
    })))
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

    if let Some((record, default_policy)) = settings {
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

        Ok(Json(serde_json::json!({
            "server_id": record.name,
            "default_policy": default_policy,
            "config": {},
            "env": masked_env,
            "auto_restart": false,
            "command": record.command,
            "args": serde_json::from_str::<Vec<String>>(&record.args).unwrap_or_default(),
            "description": record.description,
        })))
    } else {
        // Fallback: config-loaded servers not yet in DB — use in-memory env
        let servers = state.mcp_manager.list_servers().await;
        if let Some(server) = servers.iter().find(|s| s.id == name) {
            let masked_env: HashMap<String, String> = config_env
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
            Ok(Json(serde_json::json!({
                "server_id": server.id,
                "default_policy": "opt-in",
                "config": {},
                "env": masked_env,
                "auto_restart": false,
                "command": server.command,
                "args": server.args,
                "description": null,
            })))
        } else {
            Err(AppError::Validation(format!(
                "MCP server '{}' not found",
                name
            )))
        }
    }
}

/// PUT /api/mcp/servers/:name/settings
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
            // Config-loaded server (from mcp.toml) — not yet in DB.
            // Look up in-memory server info and persist it.
            let servers = state.mcp_manager.list_servers().await;
            if let Some(server) = servers.iter().find(|s| s.id == name) {
                let args_json =
                    serde_json::to_string(&server.args).unwrap_or_else(|_| "[]".to_string());
                crate::db::ensure_mcp_server_in_db(
                    &state.pool,
                    &name,
                    &server.command,
                    &args_json,
                    policy,
                )
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;
            } else {
                return Err(AppError::Validation(format!(
                    "MCP server '{}' not found",
                    name
                )));
            }
        }
    }

    // Handle env updates
    if let Some(env_obj) = body.get("env").and_then(|v| v.as_object()) {
        // Load existing env from DB to preserve unchanged values (sent as "***")
        let existing_env: HashMap<String, String> = if let Ok(Some((record, _))) =
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
            // Config-loaded server not yet in DB — persist it first
            let servers = state.mcp_manager.list_servers().await;
            if let Some(server) = servers.iter().find(|s| s.id == name) {
                let args_json =
                    serde_json::to_string(&server.args).unwrap_or_else(|_| "[]".to_string());
                crate::db::ensure_mcp_server_in_db(
                    &state.pool,
                    &name,
                    &server.command,
                    &args_json,
                    "opt-in",
                )
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;
                // Retry env update
                crate::db::update_mcp_server_env(
                    &state.pool,
                    &name,
                    &serde_json::to_string(&merged_env).unwrap_or_else(|_| "{}".to_string()),
                )
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;
            }
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

    Ok(Json(serde_json::json!({
        "status": "success",
        "message": format!("Settings updated for server '{}'", name)
    })))
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

    let default_policy = settings.as_ref().map_or("opt-in", |(_, dp)| dp.as_str());

    // Get tools from running server
    let tools: Vec<String> = {
        let servers = state.mcp_manager.list_servers().await;
        servers
            .iter()
            .find(|s| s.id == name)
            .map(|s| s.tools.clone())
            .unwrap_or_default()
    };

    Ok(Json(serde_json::json!({
        "server_id": name,
        "default_policy": default_policy,
        "tools": tools,
        "entries": entries,
    })))
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
        if !["server_grant", "tool_grant"].contains(&entry.entry_type.as_str()) {
            return Err(AppError::Validation(format!(
                "Cannot bulk-update entry_type '{}'; only server_grant and tool_grant allowed",
                entry.entry_type
            )));
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

    Ok(Json(serde_json::json!({
        "status": "success",
        "message": format!("Access control updated for server '{}'", name),
        "count": entries.len(),
    })))
}

/// GET /api/mcp/access/by-agent/:agent_id
pub async fn get_agent_access(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let entries = crate::db::get_access_entries_for_agent(&state.pool, &agent_id)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;

    Ok(Json(serde_json::json!({
        "agent_id": agent_id,
        "entries": entries,
    })))
}

/// POST /api/mcp/servers/:name/restart
pub async fn restart_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let tools = state
        .mcp_manager
        .restart_server(&name)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to restart server: {}", e)))?;

    spawn_admin_audit(
        state.pool.clone(),
        "MCP_SERVER_RESTARTED",
        name.clone(),
        "MCP server restarted".to_string(),
        None,
        None,
        None,
    );

    info!(name = %name, "🔄 MCP server restarted");

    Ok(Json(serde_json::json!({
        "status": "restarted",
        "name": name,
        "tools": tools,
    })))
}

/// POST /api/mcp/servers/:name/start
pub async fn start_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let tools = state
        .mcp_manager
        .start_server(&name)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to start server: {}", e)))?;

    spawn_admin_audit(
        state.pool.clone(),
        "MCP_SERVER_STARTED",
        name.clone(),
        "MCP server started".to_string(),
        None,
        None,
        None,
    );

    info!(name = %name, "▶️ MCP server started");

    Ok(Json(serde_json::json!({
        "status": "started",
        "name": name,
        "tools": tools,
    })))
}

/// POST /api/mcp/servers/:name/stop
pub async fn stop_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    state
        .mcp_manager
        .stop_server(&name)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to stop server: {}", e)))?;

    spawn_admin_audit(
        state.pool.clone(),
        "MCP_SERVER_STOPPED",
        name.clone(),
        "MCP server stopped".to_string(),
        None,
        None,
        None,
    );

    info!(name = %name, "⏹️ MCP server stopped");

    Ok(Json(serde_json::json!({
        "status": "stopped",
        "name": name,
    })))
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
    Ok(Json(serde_json::json!({ "enabled": enabled })))
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
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    state
        .mcp_manager
        .yolo_mode
        .store(enabled, std::sync::atomic::Ordering::Relaxed);

    if enabled {
        tracing::warn!("YOLO mode enabled via API");
    } else {
        tracing::info!("YOLO mode disabled via API");
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

    Ok(Json(
        serde_json::json!({ "status": "ok", "enabled": enabled }),
    ))
}
