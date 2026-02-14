mod client;
mod protocol;
mod stdio;

use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;
use exiv_shared::{
    Plugin, PluginConfig, exiv_plugin, ExivEvent, ExivEventData, 
    PluginRuntimeContext, Permission
};
use tracing::{info, error};
use crate::client::McpClient;

#[exiv_plugin(
    name = "adapter.mcp",
    kind = "Skill", // Tool提供が主目的
    description = "Model Context Protocol (MCP) Client Adapter. Connects external tools to Exiv.",
    version = "0.1.0",
    category = "Tool",
    config_keys = ["mcp_servers_config"],
    permissions = ["ProcessExecution"], // 外部プロセス起動のため必須
    capabilities = ["Tool"],
    tags = ["#TOOL", "#ADAPTER"]
)]
pub struct McpAdapterPlugin {
    state: Arc<RwLock<McpState>>,
}

struct McpState {
    /// Server Name -> Process/Client
    servers: HashMap<String, McpServerInstance>,
    configured: bool,
}

struct McpServerInstance {
    _name: String,
    command: String,
    args: Vec<String>,
    client: Option<Arc<McpClient>>, 
}

#[derive(serde::Deserialize)]
struct ServerConfig {
    name: String,
    command: String,
    args: Vec<String>,
}

impl McpAdapterPlugin {
    pub async fn new_plugin(config: PluginConfig) -> anyhow::Result<Self> {
        let mut servers = HashMap::new();
        let mut configured = false;

        if let Some(json_str) = config.config_values.get("mcp_servers_config") {
            match serde_json::from_str::<Vec<ServerConfig>>(json_str) {
                Ok(configs) => {
                    for cfg in configs {
                        servers.insert(cfg.name.clone(), McpServerInstance {
                            _name: cfg.name,
                            command: cfg.command,
                            args: cfg.args,
                            client: None,
                        });
                    }
                    configured = true;
                }
                Err(e) => {
                    error!("❌ Failed to parse mcp_servers_config: {}", e);
                }
            }
        }

        Ok(Self {
            state: Arc::new(RwLock::new(McpState {
                servers,
                configured,
            })),
        })
    }
}

#[async_trait]
impl Plugin for McpAdapterPlugin {
    fn manifest(&self) -> exiv_shared::PluginManifest {
        self.auto_manifest()
    }

    async fn on_plugin_init(
        &self,
        context: PluginRuntimeContext,
        _network: Option<Arc<dyn exiv_shared::NetworkCapability>>,
    ) -> anyhow::Result<()> {
        // 権限チェック
        if !context.effective_permissions.contains(&Permission::ProcessExecution) {
            error!("🚫 adapter.mcp requires ProcessExecution permission to spawn MCP servers.");
            return Ok(());
        }

        let mut state = self.state.write().await;
        if state.configured {
            info!("🔌 MCP Adapter initializing {} servers...", state.servers.len());
            
            // Connect to all configured servers
            for (name, instance) in state.servers.iter_mut() {
                info!("   - Connecting to [MCP] {}: {} {:?}", name, instance.command, instance.args);

                // H-12: Retry with exponential backoff (3 attempts)
                let mut connected = false;
                for attempt in 1..=3u32 {
                    match McpClient::connect(&instance.command, &instance.args).await {
                        Ok(client) => {
                            info!("   ✅ Connected to [MCP] {}", name);

                            match client.list_tools().await {
                                Ok(tools) => {
                                    info!("      🛠️ Found {} tools on {}", tools.tools.len(), name);
                                    for tool in tools.tools {
                                        info!("         - {}: {}", tool.name, tool.description.unwrap_or_default());
                                    }
                                }
                                Err(e) => error!("      ❌ Failed to list tools: {}", e),
                            }

                            instance.client = Some(Arc::new(client));
                            connected = true;
                            break;
                        }
                        Err(e) => {
                            if attempt < 3 {
                                let delay = std::time::Duration::from_secs(1 << (attempt - 1));
                                error!("   ⚠️ Connection attempt {}/3 failed for [MCP] {}: {}. Retrying in {:?}...", attempt, name, e, delay);
                                tokio::time::sleep(delay).await;
                            } else {
                                error!("   ❌ Failed to connect to [MCP] {} after 3 attempts: {}", name, e);
                            }
                        }
                    }
                }
                if !connected {
                    error!("   ⚠️ MCP Server '{}' is unavailable. Tool calls will fail.", name);
                }
            }
        } else {
            info!("⚠️ MCP Adapter initialized but no servers configured.");
        }
        Ok(())
    }

    async fn on_event(&self, _event: &ExivEvent) -> anyhow::Result<Option<ExivEventData>> {
        // 将来的に: ToolUse要求を受け取って処理
        Ok(None)
    }
}

#[async_trait]
impl exiv_shared::Tool for McpAdapterPlugin {
    fn name(&self) -> &str {
        "MCP Client"
    }

    fn description(&self) -> &str {
        "Executes tools provided by connected MCP servers."
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        // args: { "server": "filesystem", "tool": "read_file", "args": { ... } }
        let server_name = args.get("server").and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("Missing 'server' argument"))?;
        let tool_name = args.get("tool").and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("Missing 'tool' argument"))?;
        let tool_args = args.get("args").cloned().unwrap_or(serde_json::json!({}));

        let state = self.state.read().await;
        if let Some(instance) = state.servers.get(server_name) {
            if let Some(client) = &instance.client {
                let result = client.call_tool(tool_name, tool_args).await?;
                return Ok(serde_json::to_value(result)?);
            }
        }
        
        Err(anyhow::anyhow!("MCP Server '{}' not found or not connected.", server_name))
    }
}
