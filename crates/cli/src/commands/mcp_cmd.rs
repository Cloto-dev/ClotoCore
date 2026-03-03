use anyhow::Result;
use colored::Colorize;
use comfy_table::presets::NOTHING;
use comfy_table::{ContentArrangement, Table};
use dialoguer::{theme::ColorfulTheme, Confirm};

use crate::cli::McpCommand;
use crate::client::ClotoClient;
use crate::output;

pub async fn run(client: &ClotoClient, cmd: McpCommand, json_mode: bool) -> Result<()> {
    match cmd {
        McpCommand::List => list(client, json_mode).await,
        McpCommand::Create {
            name,
            command,
            args,
            description,
        } => {
            create(
                client,
                &name,
                &command,
                &args,
                description.as_deref(),
                json_mode,
            )
            .await
        }
        McpCommand::Delete { name, force } => delete(client, &name, force, json_mode).await,
        McpCommand::Start { name } => lifecycle(client, &name, "start", json_mode).await,
        McpCommand::Stop { name } => lifecycle(client, &name, "stop", json_mode).await,
        McpCommand::Restart { name } => lifecycle(client, &name, "restart", json_mode).await,
        McpCommand::Settings { name } => settings(client, &name, json_mode).await,
        McpCommand::Access { name } => access(client, &name, json_mode).await,
    }
}

async fn list(client: &ClotoClient, json_mode: bool) -> Result<()> {
    let sp = if json_mode {
        None
    } else {
        Some(output::spinner("Loading MCP servers..."))
    };

    let result: serde_json::Value = client.get_mcp_servers().await?;

    if let Some(sp) = sp {
        sp.finish_and_clear();
    }

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    output::print_header("MCP Servers");

    let servers = result
        .get("servers")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if servers.is_empty() {
        println!("  {}", "No MCP servers connected.".dimmed());
        println!();
        return Ok(());
    }

    let mut table = Table::new();
    table
        .load_preset(NOTHING)
        .set_content_arrangement(ContentArrangement::Dynamic);

    for srv in &servers {
        let id = srv.get("id").and_then(|v| v.as_str()).unwrap_or("-");
        let tools = srv
            .get("tools")
            .and_then(|v| v.as_array())
            .map_or(0, std::vec::Vec::len);
        let command = srv.get("command").and_then(|v| v.as_str()).unwrap_or("-");

        table.add_row(vec![
            format!("  {}", "●".green()),
            id.bold().to_string(),
            format!("{tools} tools"),
            command.dimmed().to_string(),
        ]);
    }

    println!("{table}");
    println!();
    Ok(())
}

async fn create(
    client: &ClotoClient,
    name: &str,
    command: &str,
    args: &[String],
    description: Option<&str>,
    json_mode: bool,
) -> Result<()> {
    let mut body = serde_json::json!({
        "name": name,
        "command": command,
        "args": args,
    });
    if let Some(desc) = description {
        body["description"] = serde_json::json!(desc);
    }

    let sp = if json_mode {
        None
    } else {
        Some(output::spinner("Creating MCP server..."))
    };

    let result: serde_json::Value = client.create_mcp_server(&body).await?;

    if let Some(sp) = sp {
        sp.finish_and_clear();
    }

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    let tools = result
        .get("tools")
        .and_then(|v| v.as_array())
        .map_or(0, std::vec::Vec::len);

    println!(
        "  {} MCP server created: {} ({tools} tools)",
        "✓".green().bold(),
        name.bold()
    );
    println!();
    Ok(())
}

async fn delete(client: &ClotoClient, name: &str, force: bool, json_mode: bool) -> Result<()> {
    if !force && !json_mode {
        output::print_header("Delete MCP Server");
        println!("  Server: {}", name.bold());
        println!();
        let confirmed = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("  Confirm deletion?")
            .default(false)
            .interact()?;
        if !confirmed {
            println!("  Cancelled.");
            return Ok(());
        }
    }

    let result: serde_json::Value = client.delete_mcp_server(name).await?;

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    println!(
        "  {} MCP server deleted: {}",
        "✓".green().bold(),
        name.bold()
    );
    println!();
    Ok(())
}

async fn lifecycle(client: &ClotoClient, name: &str, action: &str, json_mode: bool) -> Result<()> {
    let sp = if json_mode {
        None
    } else {
        let msg = match action {
            "start" => format!("Starting {name}..."),
            "stop" => format!("Stopping {name}..."),
            "restart" => format!("Restarting {name}..."),
            _ => format!("{action} {name}..."),
        };
        Some(output::spinner(&msg))
    };

    let result: serde_json::Value = match action {
        "start" => client.start_mcp_server(name).await?,
        "stop" => client.stop_mcp_server(name).await?,
        "restart" => client.restart_mcp_server(name).await?,
        _ => anyhow::bail!("Unknown action: {action}"),
    };

    if let Some(sp) = sp {
        sp.finish_and_clear();
    }

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    let status = result
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or(action);
    println!("  {} {} {}", "✓".green().bold(), name.bold(), status);
    println!();
    Ok(())
}

async fn settings(client: &ClotoClient, name: &str, json_mode: bool) -> Result<()> {
    let sp = if json_mode {
        None
    } else {
        Some(output::spinner("Loading settings..."))
    };

    let result: serde_json::Value = client.get_mcp_server_settings(name).await?;

    if let Some(sp) = sp {
        sp.finish_and_clear();
    }

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    output::print_header(&format!("Settings: {name}"));

    let command = result
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("-");
    let policy = result
        .get("default_policy")
        .and_then(|v| v.as_str())
        .unwrap_or("-");
    let auto_restart = result
        .get("auto_restart")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);

    println!("  Command:        {}", command);
    println!("  Default Policy: {}", policy);
    println!(
        "  Auto Restart:   {}",
        if auto_restart { "yes" } else { "no" }
    );

    if let Some(env) = result.get("env").and_then(|v| v.as_object()) {
        if !env.is_empty() {
            println!();
            println!("  {}", "Environment:".dimmed());
            for (key, val) in env {
                let val_str = val.as_str().unwrap_or("-");
                println!("    {} = {}", key, val_str.dimmed());
            }
        }
    }

    println!();
    Ok(())
}

async fn access(client: &ClotoClient, name: &str, json_mode: bool) -> Result<()> {
    let sp = if json_mode {
        None
    } else {
        Some(output::spinner("Loading access control..."))
    };

    let result: serde_json::Value = client.get_mcp_server_access(name).await?;

    if let Some(sp) = sp {
        sp.finish_and_clear();
    }

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    output::print_header(&format!("Access Control: {name}"));

    let policy = result
        .get("default_policy")
        .and_then(|v| v.as_str())
        .unwrap_or("-");
    println!("  Default Policy: {}", policy);

    if let Some(tools) = result.get("tools").and_then(|v| v.as_array()) {
        println!("  Tools: {}", tools.len());
    }

    let entries = result
        .get("entries")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if entries.is_empty() {
        println!();
        println!("  {}", "No access entries.".dimmed());
    } else {
        println!();
        let mut table = Table::new();
        table
            .load_preset(NOTHING)
            .set_content_arrangement(ContentArrangement::Dynamic);

        for entry in &entries {
            let entry_type = entry
                .get("entry_type")
                .and_then(|v| v.as_str())
                .unwrap_or("-");
            let agent = entry
                .get("agent_id")
                .and_then(|v| v.as_str())
                .unwrap_or("-");
            let tool = entry
                .get("tool_name")
                .and_then(|v| v.as_str())
                .unwrap_or("*");

            let type_colored = match entry_type {
                "server_grant" | "tool_grant" => entry_type.green().to_string(),
                "server_deny" | "tool_deny" => entry_type.red().to_string(),
                _ => entry_type.to_string(),
            };

            table.add_row(vec![
                format!("  "),
                type_colored,
                agent.to_string(),
                tool.dimmed().to_string(),
            ]);
        }

        println!("{table}");
    }

    println!();
    Ok(())
}
