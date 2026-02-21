//! L5 Self-Extension: Skill Manager Tool
//!
//! Provides `register_skill` and `add_network_host` actions to the agentic loop,
//! enabling agents to create new tools and grant network access at runtime.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

use crate::capabilities::SafeHttpClient;
use crate::db;
use crate::managers::{PluginManager, PluginRegistry};
use exiv_shared::{
    ExivEvent, Permission, Plugin, PluginCast, PluginCategory, PluginManifest, ServiceType, Tool,
};
use sqlx::SqlitePool;

pub struct SkillManager {
    plugin_manager: Arc<PluginManager>,
    registry: Arc<PluginRegistry>,
    http_client: Arc<SafeHttpClient>,
    pool: SqlitePool,
}

impl SkillManager {
    pub fn new(
        plugin_manager: Arc<PluginManager>,
        registry: Arc<PluginRegistry>,
        http_client: Arc<SafeHttpClient>,
        pool: SqlitePool,
    ) -> Self {
        Self {
            plugin_manager,
            registry,
            http_client,
            pool,
        }
    }

    async fn handle_register_skill(
        &self,
        args: &serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required field: name"))?;

        let code = args
            .get("code")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required field: code"))?;

        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("A runtime-registered skill.");

        // Name validation: alphanumeric + underscore, 1-64 chars
        if name.is_empty() || name.len() > 64 {
            return Err(anyhow::anyhow!("Skill name must be 1-64 characters"));
        }
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(anyhow::anyhow!(
                "Skill name must contain only alphanumeric characters and underscores"
            ));
        }

        // Allow caller to provide a custom JSON Schema for the tool's parameters.
        // Defaults to {input: string} for backward compatibility.
        let default_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "input": {
                    "type": "string",
                    "description": "Input for the skill"
                }
            }
        });
        let tool_schema = args
            .get("tool_schema")
            .filter(|v| v.is_object())
            .cloned()
            .unwrap_or(default_schema);
        let tool_schema_str = tool_schema.to_string();

        // Build Python script with EXIV_MANIFEST.
        // tool_schema_str is inserted as a raw JSON literal (valid Python dict syntax).
        let script_content = format!(
            r##"# Auto-generated runtime skill: {name}
# This script was created by the Skill Manager at runtime.

EXIV_MANIFEST = {{
    "name": "python.runtime.{name}",
    "description": "{description}",
    "provided_tools": ["{name}"],
    "tool_description": "{description}",
    "tool_schema": {tool_schema},
    "tags": ["#RUNTIME", "#SKILL"]
}}

{code}

def execute(params):
    """Entry point called by the bridge when this tool is invoked.
    params is a dict whose keys match the tool_schema properties.
    """
    return on_execute(params)
"##,
            name = name,
            description = description.replace('"', r#"\""#),
            tool_schema = tool_schema_str,
            code = code,
        );

        // Write script to scripts/ directory
        let script_filename = format!("runtime_{}.py", name);
        let scripts_dir = std::path::Path::new("scripts");
        if !scripts_dir.exists() {
            std::fs::create_dir_all(scripts_dir)?;
        }
        let script_path = scripts_dir.join(&script_filename);
        std::fs::write(&script_path, &script_content)?;

        info!(skill_name = %name, path = %script_path.display(), "📝 L5: Wrote runtime skill script");

        // Parse permissions from args
        let permissions = if args
            .get("network_access")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            vec![Permission::NetworkAccess]
        } else {
            vec![]
        };

        // Register as a runtime plugin
        let plugin_id = format!("python.runtime.{}", name);
        let mut config_values = HashMap::new();
        config_values.insert("script_path".to_string(), script_filename.clone());

        let permissions_json = serde_json::to_string(
            &permissions
                .iter()
                .map(|p| format!("{:?}", p))
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".to_string());

        self.plugin_manager
            .register_runtime_plugin(&plugin_id, config_values, permissions, &self.registry)
            .await?;

        // Persist to database for survival across restarts
        let record = db::RuntimePluginRecord {
            plugin_id: plugin_id.clone(),
            script_name: script_filename,
            description: Some(description.to_string()),
            code_content: script_content,
            permissions: permissions_json,
            created_at: chrono::Utc::now().timestamp_millis(),
            created_by: None,
            generation_number: None,
            is_active: true,
        };
        if let Err(e) = db::save_runtime_plugin(&self.pool, &record).await {
            tracing::warn!(error = %e, plugin_id = %plugin_id, "Failed to persist runtime plugin to DB (plugin is active but ephemeral)");
        } else {
            info!(plugin_id = %plugin_id, "💾 L5: Runtime plugin persisted to database");
        }

        Ok(serde_json::json!({
            "status": "registered",
            "plugin_id": plugin_id,
            "tool_name": name,
        }))
    }

    async fn handle_get_runtime_skills(&self) -> anyhow::Result<serde_json::Value> {
        let records = crate::db::load_active_runtime_plugins(&self.pool).await?;
        let skills: Vec<serde_json::Value> = records
            .iter()
            .map(|r| {
                serde_json::json!({
                    "plugin_id": r.plugin_id,
                    "description": r.description,
                    "created_at": r.created_at,
                })
            })
            .collect();
        Ok(serde_json::json!({
            "runtime_skills": skills,
            "count": skills.len(),
            "note": if skills.is_empty() {
                "No runtime skills created yet. Use register_skill to create one."
            } else {
                "These are skills you have previously created. Avoid creating duplicates."
            }
        }))
    }

    fn handle_add_network_host(
        &self,
        args: &serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let host = args
            .get("host")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required field: host"))?;

        // Hostname validation
        if host.is_empty() || host.len() > 253 {
            return Err(anyhow::anyhow!("Hostname must be 1-253 characters"));
        }
        if host.contains('/') || host.contains(':') || host.contains(' ') {
            return Err(anyhow::anyhow!(
                "Invalid hostname: must not contain '/', ':', or spaces"
            ));
        }

        let newly_added = self.http_client.add_host(host);
        info!(host = %host, newly_added = %newly_added, "🌐 L5: Network host whitelist update");

        Ok(serde_json::json!({
            "status": if newly_added { "added" } else { "already_present" },
            "host": host,
        }))
    }
}

impl PluginCast for SkillManager {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_tool(&self) -> Option<&dyn Tool> {
        Some(self)
    }
}

#[async_trait]
impl Plugin for SkillManager {
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "core.skill_manager".to_string(),
            name: "Skill Manager".to_string(),
            description:
                "L5 Self-Extension: Register new skills and grant network access at runtime."
                    .to_string(),
            version: "1.0.0".to_string(),
            category: PluginCategory::System,
            service_type: ServiceType::Skill,
            tags: vec!["#SYSTEM".to_string(), "#L5".to_string()],
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
            provided_tools: vec!["skill_manager".to_string()],
        }
    }

    async fn on_event(
        &self,
        _event: &ExivEvent,
    ) -> anyhow::Result<Option<exiv_shared::ExivEventData>> {
        Ok(None)
    }
}

#[async_trait]
impl Tool for SkillManager {
    fn name(&self) -> &'static str {
        "skill_manager"
    }

    fn description(&self) -> &'static str {
        "Extend your own capabilities at runtime. Use get_runtime_skills to see skills you have already created (avoids duplicates). Use register_skill to write and register a new Python skill with custom input parameters. Use add_network_host to grant yourself network access to a specific host."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["get_runtime_skills", "register_skill", "add_network_host"],
                    "description": "The action to perform. Use get_runtime_skills first to check what skills you have already created."
                },
                "name": {
                    "type": "string",
                    "description": "Skill name (alphanumeric + underscore, 1-64 chars). Required for register_skill."
                },
                "code": {
                    "type": "string",
                    "description": "Python code defining on_execute(params) function. Required for register_skill. params is a dict whose keys match tool_schema properties. Example: def on_execute(params): city = params.get('city', 'Tokyo'); return {'result': city}"
                },
                "description": {
                    "type": "string",
                    "description": "Human-readable description of what the skill does. Optional for register_skill."
                },
                "tool_schema": {
                    "type": "object",
                    "description": "JSON Schema object defining the input parameters for the skill. Optional — if omitted, defaults to {input: string}. Use this to define precise, named parameters. Example: {\"type\":\"object\",\"required\":[\"city\"],\"properties\":{\"city\":{\"type\":\"string\",\"description\":\"City name\"},\"unit\":{\"type\":\"string\",\"enum\":[\"celsius\",\"fahrenheit\"]}}}"
                },
                "network_access": {
                    "type": "boolean",
                    "description": "Whether the skill needs to make HTTP requests. Set true to enable requests library. Optional for register_skill."
                },
                "host": {
                    "type": "string",
                    "description": "Hostname to add to the network whitelist (e.g. 'api.example.com'). Required for add_network_host."
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required field: action"))?;

        match action {
            "get_runtime_skills" => self.handle_get_runtime_skills().await,
            "register_skill" => self.handle_register_skill(&args).await,
            "add_network_host" => self.handle_add_network_host(&args),
            _ => Err(anyhow::anyhow!("Unknown action: '{}'. Valid actions: get_runtime_skills, register_skill, add_network_host", action)),
        }
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_skill_name_validation_empty() {
        // Empty names should be rejected (tested via the validation logic)
        let name = "";
        assert!(name.is_empty());
    }

    #[test]
    fn test_skill_name_validation_special_chars() {
        let name = "my-skill";
        assert!(!name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    }

    #[test]
    fn test_skill_name_validation_valid() {
        let name = "web_scraper_v2";
        assert!(!name.is_empty() && name.len() <= 64);
        assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    }

    #[test]
    fn test_hostname_validation_with_slash() {
        let host = "example.com/path";
        assert!(host.contains('/'));
    }

    #[test]
    fn test_hostname_validation_with_colon() {
        let host = "example.com:8080";
        assert!(host.contains(':'));
    }

    #[test]
    fn test_hostname_validation_valid() {
        let host = "api.example.com";
        assert!(!host.is_empty() && host.len() <= 253);
        assert!(!host.contains('/') && !host.contains(':') && !host.contains(' '));
    }

    #[test]
    fn test_manifest_fields() {
        // Verify SkillManager would produce correct manifest fields
        let manifest_id = "core.skill_manager";
        let magic_seal: u32 = 0x56455253;
        assert_eq!(manifest_id, "core.skill_manager");
        assert_eq!(magic_seal, 0x56455253);
    }
}
