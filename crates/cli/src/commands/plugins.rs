use anyhow::Result;

use crate::cli::PluginsCommand;
use crate::client::ExivClient;
use crate::output;

pub async fn run(client: &ExivClient, cmd: PluginsCommand, json_mode: bool) -> Result<()> {
    match cmd {
        PluginsCommand::List => list(client, json_mode).await,
    }
}

async fn list(client: &ExivClient, json_mode: bool) -> Result<()> {
    let sp = if !json_mode { Some(output::spinner("Loading plugins...")) } else { None };
    let plugins = client.get_plugins().await?;
    if let Some(sp) = sp { sp.finish_and_clear(); }

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&plugins)?);
        return Ok(());
    }

    output::print_header("Loaded Plugins");
    output::print_plugins_table(&plugins);
    println!();
    Ok(())
}
