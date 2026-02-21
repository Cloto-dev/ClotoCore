use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "exiv",
    about = "Exiv — AI Agent Management CLI",
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
