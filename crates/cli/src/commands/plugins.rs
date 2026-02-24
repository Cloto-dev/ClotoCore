use anyhow::Result;

use crate::cli::PluginsCommand;
use crate::client::ClotoClient;
use crate::output;

pub async fn run(client: &ClotoClient, cmd: PluginsCommand, json_mode: bool) -> Result<()> {
    match cmd {
        PluginsCommand::List => list(client, json_mode).await,
    }
}

async fn list(client: &ClotoClient, json_mode: bool) -> Result<()> {
    let sp = if json_mode {
        None
    } else {
        Some(output::spinner("Loading plugins..."))
    };
    let plugins = client.get_plugins().await?;
    if let Some(sp) = sp {
        sp.finish_and_clear();
    }

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&plugins)?);
        return Ok(());
    }

    output::print_header("Loaded Plugins");
    output::print_plugins_table(&plugins);
    println!();
    Ok(())
}
