pub mod status;
pub mod agents;
pub mod plugins;
pub mod chat;
pub mod config_cmd;

use anyhow::Result;
use crate::cli::*;
use crate::client::ExivClient;
use crate::config::CliConfig;

pub async fn dispatch(cli: Cli) -> Result<()> {
    let config = CliConfig::load()?;
    let client = ExivClient::new(&config);

    match cli.command {
        Commands::Status => status::run(&client, cli.json).await,
        Commands::Agents(cmd) => agents::run(&client, cmd, cli.json).await,
        Commands::Plugins(cmd) => plugins::run(&client, cmd, cli.json).await,
        Commands::Chat { agent, message } => {
            let msg = message.join(" ");
            chat::run(&client, &agent, &msg, cli.json).await
        }
        Commands::Logs { .. } => {
            anyhow::bail!("Logs command will be available in Phase 3")
        }
        Commands::Config(cmd) => config_cmd::run(cmd, &config),
    }
}
