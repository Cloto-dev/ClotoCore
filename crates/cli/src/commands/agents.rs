use anyhow::Result;
use colored::Colorize;
use dialoguer::{theme::ColorfulTheme, Confirm, Input, Password, Select};

use crate::cli::AgentsCommand;
use crate::client::ClotoClient;
use crate::output;

pub async fn run(client: &ClotoClient, cmd: AgentsCommand, json_mode: bool) -> Result<()> {
    match cmd {
        AgentsCommand::List => list(client, json_mode).await,
        AgentsCommand::Create {
            name,
            description,
            engine,
            agent_type,
            password,
        } => {
            create(
                client,
                name,
                description,
                engine,
                agent_type,
                password,
                json_mode,
            )
            .await
        }
        AgentsCommand::Power {
            agent,
            on,
            off,
            password,
        } => {
            let enabled = if on {
                true
            } else if off {
                false
            } else {
                anyhow::bail!("Specify --on or --off");
            };
            power(client, &agent, enabled, password, json_mode).await
        }
        AgentsCommand::Delete { agent, force } => delete(client, &agent, force, json_mode).await,
    }
}

async fn list(client: &ClotoClient, json_mode: bool) -> Result<()> {
    let sp = if json_mode {
        None
    } else {
        Some(output::spinner("Loading agents..."))
    };
    let agents = client.get_agents().await?;
    if let Some(sp) = sp {
        sp.finish_and_clear();
    }

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&agents)?);
        return Ok(());
    }

    output::print_header("Registered Agents");
    output::print_agents_table(&agents);
    println!();
    Ok(())
}

async fn create(
    client: &ClotoClient,
    name: Option<String>,
    description: Option<String>,
    engine: Option<String>,
    agent_type: Option<String>,
    password: Option<String>,
    json_mode: bool,
) -> Result<()> {
    // If all required flags are provided, skip interactive mode
    let has_all_flags = name.is_some() && engine.is_some();

    let (name, description, engine, agent_type, password) = if has_all_flags {
        (
            name.unwrap(),
            description.unwrap_or_default(),
            engine.unwrap(),
            agent_type,
            password,
        )
    } else {
        // Interactive wizard
        interactive_create_wizard(client).await?
    };

    let description = if description.is_empty() {
        format!("Agent: {name}")
    } else {
        description
    };

    let mut metadata = std::collections::HashMap::new();
    if let Some(ref at) = agent_type {
        metadata.insert("agent_type".to_string(), at.clone());
    }

    let body = serde_json::json!({
        "name": name,
        "description": description,
        "default_engine": engine,
        "metadata": metadata,
        "password": password,
    });

    let sp = if json_mode {
        None
    } else {
        Some(output::spinner("Creating agent..."))
    };
    let result: serde_json::Value = client.create_agent(&body).await?;
    if let Some(sp) = sp {
        sp.finish_and_clear();
    }

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    let agent_id = result
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    println!(
        "  {} Agent created: {}",
        "✓".green().bold(),
        agent_id.bold()
    );
    println!();
    Ok(())
}

/// Interactive agent creation wizard using dialoguer.
async fn interactive_create_wizard(
    client: &ClotoClient,
) -> Result<(String, String, String, Option<String>, Option<String>)> {
    let theme = ColorfulTheme::default();

    output::print_header("Create New Agent");

    // 1. Agent type selection
    let type_items = &[
        "AI Agent      (reasoning engine)",
        "Container     (external process)",
    ];
    let type_idx = Select::with_theme(&theme)
        .with_prompt("  Agent type")
        .items(type_items)
        .default(0)
        .interact()?;
    let agent_type = if type_idx == 0 { "ai" } else { "container" };

    // 2. Name
    let name: String = Input::with_theme(&theme)
        .with_prompt("  Name")
        .interact_text()?;

    // 3. Description
    let description: String = Input::with_theme(&theme)
        .with_prompt("  Description")
        .default(format!("Agent: {name}"))
        .interact_text()?;

    // 4. Engine selection — fetch available engines from plugins
    let engine = select_engine(client, agent_type).await?;

    // 5. Optional password
    let password_input = Password::with_theme(&theme)
        .with_prompt("  Power password (optional, Enter to skip)")
        .allow_empty_password(true)
        .interact()?;
    let password = if password_input.is_empty() {
        None
    } else {
        Some(password_input)
    };

    Ok((
        name,
        description,
        engine,
        Some(agent_type.to_string()),
        password,
    ))
}

/// Fetch plugins and let user select an engine.
async fn select_engine(client: &ClotoClient, agent_type: &str) -> Result<String> {
    let theme = ColorfulTheme::default();

    // Fetch plugins to show available engines
    let plugins = client.get_plugins().await.unwrap_or_default();

    let engines: Vec<(&str, &str)> = plugins
        .iter()
        .filter(|p| {
            if agent_type == "ai" {
                matches!(p.category, cloto_shared::PluginCategory::Agent)
                    && p.id.starts_with("mind.")
            } else {
                // Container agents can use any non-mind engine
                !p.id.starts_with("mind.")
                    && !matches!(p.category, cloto_shared::PluginCategory::System)
            }
        })
        .map(|p| (p.id.as_str(), p.name.as_str()))
        .collect();

    if engines.is_empty() {
        // No matching engines found, fall back to manual input
        let engine: String = Input::with_theme(&theme)
            .with_prompt("  Engine ID")
            .interact_text()?;
        return Ok(engine);
    }

    let items: Vec<String> = engines
        .iter()
        .map(|(id, name)| format!("{id:<20} ({name})"))
        .collect();

    let idx = Select::with_theme(&theme)
        .with_prompt("  Engine")
        .items(&items)
        .default(0)
        .interact()?;

    Ok(engines[idx].0.to_string())
}

async fn power(
    client: &ClotoClient,
    agent_id: &str,
    enabled: bool,
    password: Option<String>,
    json_mode: bool,
) -> Result<()> {
    // If the agent has a password and none was provided, prompt interactively
    let sp = if json_mode {
        None
    } else {
        Some(output::spinner(&format!(
            "Powering {} {}...",
            agent_id,
            if enabled { "ON" } else { "OFF" }
        )))
    };

    let result = client
        .power_toggle(agent_id, enabled, password.as_deref())
        .await;
    if let Some(ref sp) = sp {
        sp.finish_and_clear();
    }

    match result {
        Ok(result) => {
            if json_mode {
                println!("{}", serde_json::to_string_pretty(&result)?);
                return Ok(());
            }
            println!(
                "  {} {agent_id} powered {}",
                "✓".green().bold(),
                if enabled {
                    "ON".green().bold()
                } else {
                    "OFF".red().bold()
                },
            );
            println!();
            Ok(())
        }
        Err(e)
            if e.to_string().contains("Password required")
                || e.to_string().contains("password") =>
        {
            if json_mode {
                return Err(e);
            }
            // Prompt for password and retry
            let pw = Password::with_theme(&ColorfulTheme::default())
                .with_prompt("  Password required")
                .interact()?;

            let sp = output::spinner(&format!(
                "Powering {} {}...",
                agent_id,
                if enabled { "ON" } else { "OFF" }
            ));
            let result = client.power_toggle(agent_id, enabled, Some(&pw)).await?;
            sp.finish_and_clear();

            println!(
                "  {} {agent_id} powered {}",
                "✓".green().bold(),
                if enabled {
                    "ON".green().bold()
                } else {
                    "OFF".red().bold()
                },
            );
            println!();

            if json_mode {
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            Ok(())
        }
        Err(e) => Err(e),
    }
}

async fn delete(client: &ClotoClient, agent_id: &str, force: bool, json_mode: bool) -> Result<()> {
    if !force && !json_mode {
        output::print_header("Delete Agent");
        println!("  Agent:   {}", agent_id.bold());
        println!("  {}", "⚠  This action is irreversible.".yellow().bold());
        println!("  All chat history for this agent will be permanently deleted.");
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

    let sp = if json_mode {
        None
    } else {
        Some(output::spinner("Deleting agent..."))
    };
    let result = client.delete_agent(agent_id).await;
    if let Some(sp) = sp {
        sp.finish_and_clear();
    }

    match result {
        Ok(body) => {
            if json_mode {
                println!("{}", serde_json::to_string_pretty(&body)?);
                return Ok(());
            }
            println!(
                "  {} Agent deleted: {}",
                "✓".green().bold(),
                agent_id.bold()
            );
            println!();
            Ok(())
        }
        Err(e) => {
            if json_mode {
                return Err(e);
            }
            eprintln!("  {} {}", "✗".red().bold(), e);
            std::process::exit(1);
        }
    }
}
