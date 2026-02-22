mod client;
mod protocol;
mod stdio;

use crate::client::McpClient;
use async_trait::async_trait;
use exiv_shared::{
    exiv_plugin, ExivEvent, ExivEventData, Permission, Plugin, PluginConfig, PluginRuntimeContext,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};

#[exiv_plugin(
    name = "adapter.mcp",
    kind = "Skill", // Tool提供が主目的
    description = "Model Context Protocol (MCP) Client Adapter. Connects external tools to Exiv.",
    version = "0.2.0",
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
    tools: Vec<String>,
}

#[derive(serde::Deserialize)]
struct ServerConfig {
    name: String,
    command: String,
    args: Vec<String>,
}

/// Information about a connected MCP server.
#[derive(serde::Serialize, Clone)]
pub struct ServerInfo {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub connected: bool,
    pub tools: Vec<String>,
}

impl McpAdapterPlugin {
    pub async fn new_plugin(config: PluginConfig) -> anyhow::Result<Self> {
        let mut servers = HashMap::new();
        let mut configured = false;

        if let Some(json_str) = config.config_values.get("mcp_servers_config") {
            match serde_json::from_str::<Vec<ServerConfig>>(json_str) {
                Ok(configs) => {
                    for cfg in configs {
                        servers.insert(
                            cfg.name.clone(),
                            McpServerInstance {
                                _name: cfg.name,
                                command: cfg.command,
                                args: cfg.args,
                                client: None,
                                tools: Vec::new(),
                            },
                        );
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

    /// Connect to an MCP server instance with retry logic.
    /// Returns the list of tool names on success.
    async fn connect_server(instance: &mut McpServerInstance) -> anyhow::Result<Vec<String>> {
        let mut tool_names = Vec::new();

        // H-12: Retry with exponential backoff (3 attempts)
        for attempt in 1..=3u32 {
            match McpClient::connect(&instance.command, &instance.args).await {
                Ok(client) => {
                    match client.list_tools().await {
                        Ok(tools) => {
                            tool_names = tools
                                .tools
                                .iter()
                                .map(|t| t.name.clone())
                                .collect();
                            info!(
                                "   🛠️ Found {} tools on {}",
                                tools.tools.len(),
                                instance._name
                            );
                            for tool in &tools.tools {
                                info!(
                                    "      - {}: {}",
                                    tool.name,
                                    tool.description.as_deref().unwrap_or_default()
                                );
                            }
                        }
                        Err(e) => error!("   ❌ Failed to list tools: {}", e),
                    }

                    instance.client = Some(Arc::new(client));
                    instance.tools = tool_names.clone();
                    return Ok(tool_names);
                }
                Err(e) => {
                    if attempt < 3 {
                        let delay = std::time::Duration::from_secs(1 << (attempt - 1));
                        error!(
                            "   ⚠️ Connection attempt {}/3 failed for [MCP] {}: {}. Retrying in {:?}...",
                            attempt, instance._name, e, delay
                        );
                        tokio::time::sleep(delay).await;
                    } else {
                        return Err(anyhow::anyhow!(
                            "Failed to connect to MCP server '{}' after 3 attempts: {}",
                            instance._name,
                            e
                        ));
                    }
                }
            }
        }

        unreachable!()
    }

    /// Add and connect a new MCP server at runtime.
    /// Returns the list of tool names provided by the server.
    pub async fn add_server(
        &self,
        name: String,
        command: String,
        args: Vec<String>,
    ) -> anyhow::Result<Vec<String>> {
        // Validate command against whitelist
        stdio::validate_command(&command)?;

        let mut state = self.state.write().await;

        // Duplicate check
        if state.servers.contains_key(&name) {
            return Err(anyhow::anyhow!(
                "MCP server '{}' is already registered",
                name
            ));
        }

        let mut instance = McpServerInstance {
            _name: name.clone(),
            command: command.clone(),
            args: args.clone(),
            client: None,
            tools: Vec::new(),
        };

        info!("🔌 Connecting to dynamic MCP server [{}]: {} {:?}", name, command, args);
        let tool_names = Self::connect_server(&mut instance).await?;

        state.servers.insert(name.clone(), instance);
        state.configured = true;

        info!("✅ Dynamic MCP server '{}' connected with {} tools", name, tool_names.len());
        Ok(tool_names)
    }

    /// Remove and disconnect an MCP server.
    pub async fn remove_server(&self, name: &str) -> anyhow::Result<()> {
        let mut state = self.state.write().await;
        if state.servers.remove(name).is_some() {
            info!("🗑️ MCP server '{}' removed", name);
            Ok(())
        } else {
            Err(anyhow::anyhow!("MCP server '{}' not found", name))
        }
    }

    /// List all registered MCP servers with connection status.
    pub async fn list_servers(&self) -> Vec<ServerInfo> {
        let state = self.state.read().await;
        state
            .servers
            .values()
            .map(|inst| ServerInfo {
                name: inst._name.clone(),
                command: inst.command.clone(),
                args: inst.args.clone(),
                connected: inst.client.is_some(),
                tools: inst.tools.clone(),
            })
            .collect()
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
        if !context
            .effective_permissions
            .contains(&Permission::ProcessExecution)
        {
            error!("🚫 adapter.mcp requires ProcessExecution permission to spawn MCP servers.");
            return Ok(());
        }

        let mut state = self.state.write().await;
        if state.configured {
            info!(
                "🔌 MCP Adapter initializing {} servers...",
                state.servers.len()
            );

            // Connect to all configured servers
            for (name, instance) in state.servers.iter_mut() {
                info!(
                    "   - Connecting to [MCP] {}: {} {:?}",
                    name, instance.command, instance.args
                );

                match Self::connect_server(instance).await {
                    Ok(_) => {
                        info!("   ✅ Connected to [MCP] {}", name);
                    }
                    Err(e) => {
                        error!("   ❌ {}", e);
                        error!(
                            "   ⚠️ MCP Server '{}' is unavailable. Tool calls will fail.",
                            name
                        );
                    }
                }
            }
        } else {
            info!("⚠️ MCP Adapter initialized but no servers configured.");
        }
        Ok(())
    }

    async fn on_event(&self, _event: &ExivEvent) -> anyhow::Result<Option<ExivEventData>> {
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
        let server_name = args
            .get("server")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'server' argument"))?;
        let tool_name = args
            .get("tool")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'tool' argument"))?;
        let tool_args = args.get("args").cloned().unwrap_or(serde_json::json!({}));

        let state = self.state.read().await;
        if let Some(instance) = state.servers.get(server_name) {
            if let Some(client) = &instance.client {
                let result = client.call_tool(tool_name, tool_args).await?;
                return Ok(serde_json::to_value(result)?);
            }
        }

        Err(anyhow::anyhow!(
            "MCP Server '{}' not found or not connected.",
            server_name
        ))
    }
}
