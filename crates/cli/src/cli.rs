use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "cloto",
    about = "Cloto — AI Agent Management CLI",
    version,
    propagate_version = true
)]
pub struct Cli {
    /// Output raw JSON (for scripting/piping)
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Show system status and health
    Status,

    /// Manage agents
    #[command(subcommand)]
    Agents(AgentsCommand),

    /// Manage plugins
    #[command(subcommand)]
    Plugins(PluginsCommand),

    /// Send a chat message to an agent
    Chat {
        /// Target agent ID
        agent: String,
        /// Message content
        message: Vec<String>,
    },

    /// View event logs
    Logs {
        /// Follow mode: stream events in real-time
        #[arg(short, long)]
        follow: bool,
        /// Limit number of history entries
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },

    /// Manage CLI configuration
    #[command(subcommand)]
    Config(ConfigCommand),

    /// Manage plugin permissions (Human-in-the-loop)
    #[command(subcommand)]
    Permissions(PermissionsCommand),

    /// Launch interactive TUI dashboard
    Tui,

    /// Manage MCP servers
    #[command(subcommand)]
    Mcp(McpCommand),

    /// Manage cron jobs
    #[command(subcommand)]
    Cron(CronCommand),

    /// Manage LLM providers
    #[command(subcommand)]
    Llm(LlmCommand),

    /// System operations
    #[command(subcommand)]
    System(SystemCommand),

    /// View stored memories
    Memories,

    /// View episode archives
    Episodes,
}

#[derive(Subcommand)]
pub enum AgentsCommand {
    /// List all agents
    List,
    /// Create a new agent
    Create {
        /// Agent name (skip interactive prompt)
        #[arg(long)]
        name: Option<String>,
        /// Description
        #[arg(long)]
        description: Option<String>,
        /// Default engine ID
        #[arg(long)]
        engine: Option<String>,
        /// Agent type: ai or container
        #[arg(long, value_name = "TYPE")]
        agent_type: Option<String>,
        /// Power password (optional)
        #[arg(long)]
        password: Option<String>,
    },
    /// Toggle agent power
    Power {
        /// Agent ID
        agent: String,
        /// Power on
        #[arg(long, conflicts_with = "off")]
        on: bool,
        /// Power off
        #[arg(long, conflicts_with = "on")]
        off: bool,
        /// Password (if required)
        #[arg(long)]
        password: Option<String>,
    },
    /// Delete an agent and all its data (irreversible)
    Delete {
        /// Agent ID to delete
        agent: String,
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
pub enum PluginsCommand {
    /// List all plugins
    List,
    /// Get or set plugin configuration
    #[command(subcommand)]
    Config(PluginConfigCommand),
}

#[derive(Subcommand)]
pub enum PluginConfigCommand {
    /// Show plugin configuration
    Get {
        /// Plugin ID
        plugin: String,
    },
    /// Update a plugin configuration value
    Set {
        /// Plugin ID
        plugin: String,
        /// Configuration key
        #[arg(long)]
        key: String,
        /// Configuration value
        #[arg(long)]
        value: String,
    },
}

#[derive(Subcommand)]
pub enum McpCommand {
    /// List connected MCP servers
    List,
    /// Create a dynamic MCP server
    Create {
        /// Server name (alphanumeric, underscore, hyphen)
        #[arg(long)]
        name: String,
        /// Command to run
        #[arg(long)]
        command: String,
        /// Command arguments
        #[arg(long)]
        args: Vec<String>,
        /// Server description
        #[arg(long)]
        description: Option<String>,
    },
    /// Delete an MCP server
    Delete {
        /// Server name
        name: String,
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
    /// Start a stopped MCP server
    Start {
        /// Server name
        name: String,
    },
    /// Stop a running MCP server
    Stop {
        /// Server name
        name: String,
    },
    /// Restart an MCP server
    Restart {
        /// Server name
        name: String,
    },
    /// Show MCP server settings
    Settings {
        /// Server name
        name: String,
    },
    /// Show MCP server access control
    Access {
        /// Server name
        name: String,
    },
}

#[derive(Subcommand)]
pub enum CronCommand {
    /// List cron jobs
    List {
        /// Filter by agent ID
        #[arg(long)]
        agent_id: Option<String>,
    },
    /// Create a cron job
    Create {
        /// Target agent ID
        #[arg(long)]
        agent_id: String,
        /// Job name
        #[arg(long)]
        name: String,
        /// Schedule type: interval, cron, or once
        #[arg(long)]
        schedule_type: String,
        /// Schedule value (e.g., "300" for interval, "0 */6 * * *" for cron)
        #[arg(long)]
        schedule_value: String,
        /// Message to dispatch
        #[arg(long)]
        message: String,
        /// Engine ID override
        #[arg(long)]
        engine_id: Option<String>,
        /// Max agentic iterations
        #[arg(long)]
        max_iterations: Option<u32>,
        /// Agent-speak mode: hide cron prompt, show only agent response
        #[arg(long)]
        hide_prompt: bool,
    },
    /// Delete a cron job
    Delete {
        /// Job ID
        id: String,
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
    /// Enable or disable a cron job
    Toggle {
        /// Job ID
        id: String,
        /// Enable the job
        #[arg(long, conflicts_with = "disable")]
        enable: bool,
        /// Disable the job
        #[arg(long, conflicts_with = "enable")]
        disable: bool,
    },
    /// Run a cron job immediately
    Run {
        /// Job ID
        id: String,
    },
}

#[derive(Subcommand)]
pub enum LlmCommand {
    /// List LLM providers
    List,
    /// Set API key for a provider
    SetKey {
        /// Provider ID
        provider: String,
        /// API key (omit for interactive prompt)
        #[arg(long)]
        key: Option<String>,
    },
    /// Remove API key from a provider
    DeleteKey {
        /// Provider ID
        provider: String,
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
pub enum SystemCommand {
    /// Show kernel version
    Version,
    /// Health check
    Health,
    /// Graceful shutdown
    Shutdown {
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
    /// Revoke current API key
    InvalidateKey {
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
    /// Toggle YOLO mode (skip permission prompts)
    Yolo {
        /// Set YOLO mode (omit to show current state)
        #[arg(long)]
        enable: Option<bool>,
    },
}

#[derive(Subcommand)]
pub enum PermissionsCommand {
    /// List pending permission requests
    Pending,
    /// Show current permissions for a plugin
    List {
        /// Plugin ID
        plugin: String,
    },
    /// Approve a permission request
    Approve {
        /// Request ID to approve
        request_id: String,
    },
    /// Deny a permission request
    Deny {
        /// Request ID to deny
        request_id: String,
    },
    /// Grant a permission directly to a plugin
    Grant {
        /// Plugin ID
        plugin: String,
        /// Permission to grant (NetworkAccess, FileRead, FileWrite, ProcessExecution, VisionRead, AdminAccess, MemoryRead, MemoryWrite, InputControl)
        permission: String,
    },
    /// Revoke a permission from a plugin
    Revoke {
        /// Plugin ID
        plugin: String,
        /// Permission to revoke
        permission: String,
    },
}

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Show current configuration
    Show,
    /// Set a configuration value
    Set {
        /// Key name (url, api_key)
        key: String,
        /// Value to set
        value: String,
    },
}
