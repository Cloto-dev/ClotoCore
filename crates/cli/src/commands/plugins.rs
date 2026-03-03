use anyhow::Result;
use colored::Colorize;

use crate::cli::{PluginConfigCommand, PluginsCommand};
use crate::client::ClotoClient;
use crate::output;

pub async fn run(client: &ClotoClient, cmd: PluginsCommand, json_mode: bool) -> Result<()> {
    match cmd {
        PluginsCommand::List => list(client, json_mode).await,
        PluginsCommand::Config(sub) => config(client, sub, json_mode).await,
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

async fn config(client: &ClotoClient, cmd: PluginConfigCommand, json_mode: bool) -> Result<()> {
    match cmd {
        PluginConfigCommand::Get { plugin } => {
            let sp = if json_mode {
                None
            } else {
                Some(output::spinner("Loading plugin config..."))
            };

            let result: serde_json::Value = client.get_plugin_config(&plugin).await?;

            if let Some(sp) = sp {
                sp.finish_and_clear();
            }

            if json_mode {
                println!("{}", serde_json::to_string_pretty(&result)?);
                return Ok(());
            }

            output::print_header(&format!("Config: {plugin}"));

            if let Some(obj) = result.as_object() {
                if obj.is_empty() {
                    println!("  {}", "No configuration entries.".dimmed());
                } else {
                    for (key, val) in obj {
                        println!("  {} = {}", key.bold(), val);
                    }
                }
            } else {
                println!("  {}", result);
            }
            println!();
            Ok(())
        }
        PluginConfigCommand::Set { plugin, key, value } => {
            let sp = if json_mode {
                None
            } else {
                Some(output::spinner("Updating plugin config..."))
            };

            let result: serde_json::Value =
                client.update_plugin_config(&plugin, &key, &value).await?;

            if let Some(sp) = sp {
                sp.finish_and_clear();
            }

            if json_mode {
                println!("{}", serde_json::to_string_pretty(&result)?);
                return Ok(());
            }

            println!(
                "  {} {}.{} = {}",
                "✓".green().bold(),
                plugin,
                key.bold(),
                value
            );
            println!();
            Ok(())
        }
    }
}
