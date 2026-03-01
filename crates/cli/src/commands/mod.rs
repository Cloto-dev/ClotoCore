pub mod agents;
pub mod chat;
pub mod config_cmd;
pub mod cron;
pub mod episodes;
pub mod llm;
pub mod logs;
pub mod mcp_cmd;
pub mod memories;
pub mod permissions;
pub mod plugins;
pub mod status;
pub mod system;

use crate::cli::{Cli, Commands};
use crate::client::ClotoClient;
use crate::config::CliConfig;
use anyhow::Result;

pub async fn dispatch(cli: Cli) -> Result<()> {
    let config = CliConfig::load()?;
    let client = ClotoClient::new(&config);

    match cli.command {
        Commands::Status => status::run(&client, cli.json).await,
        Commands::Agents(cmd) => agents::run(&client, cmd, cli.json).await,
        Commands::Plugins(cmd) => plugins::run(&client, cmd, cli.json).await,
        Commands::Chat { agent, message } => {
            let msg = message.join(" ");
            chat::run(&client, &agent, &msg, cli.json).await
        }
        Commands::Logs { follow, limit } => logs::run(&client, follow, limit, cli.json).await,
        Commands::Config(cmd) => config_cmd::run(cmd, &config),
        Commands::Permissions(cmd) => permissions::run(&client, cmd, cli.json).await,
        Commands::Tui => crate::tui::run().await,
        Commands::Mcp(cmd) => mcp_cmd::run(&client, cmd, cli.json).await,
        Commands::Cron(cmd) => cron::run(&client, cmd, cli.json).await,
        Commands::Llm(cmd) => llm::run(&client, cmd, cli.json).await,
        Commands::System(cmd) => system::run(&client, cmd, cli.json).await,
        Commands::Memories => memories::run(&client, cli.json).await,
        Commands::Episodes => episodes::run(&client, cli.json).await,
    }
}
