use anyhow::Result;
use colored::Colorize;
use comfy_table::presets::NOTHING;
use comfy_table::{ContentArrangement, Table};
use dialoguer::{theme::ColorfulTheme, Confirm, Password};

use crate::cli::LlmCommand;
use crate::client::ClotoClient;
use crate::output;

pub async fn run(client: &ClotoClient, cmd: LlmCommand, json_mode: bool) -> Result<()> {
    match cmd {
        LlmCommand::List => list(client, json_mode).await,
        LlmCommand::SetKey { provider, key } => set_key(client, &provider, key, json_mode).await,
        LlmCommand::DeleteKey { provider, force } => {
            delete_key(client, &provider, force, json_mode).await
        }
    }
}

async fn list(client: &ClotoClient, json_mode: bool) -> Result<()> {
    let sp = if json_mode {
        None
    } else {
        Some(output::spinner("Loading LLM providers..."))
    };

    let result: serde_json::Value = client.list_llm_providers().await?;

    if let Some(sp) = sp {
        sp.finish_and_clear();
    }

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    output::print_header("LLM Providers");

    let providers = result
        .get("providers")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if providers.is_empty() {
        println!("  {}", "No LLM providers configured.".dimmed());
        println!();
        return Ok(());
    }

    let mut table = Table::new();
    table
        .load_preset(NOTHING)
        .set_content_arrangement(ContentArrangement::Dynamic);

    for p in &providers {
        let id = p.get("id").and_then(|v| v.as_str()).unwrap_or("-");
        let display = p.get("display_name").and_then(|v| v.as_str()).unwrap_or(id);
        let has_key = p
            .get("has_key")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let enabled = p
            .get("enabled")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);

        let key_status = if has_key {
            "set".green().to_string()
        } else {
            "missing".red().to_string()
        };

        let enabled_dot = if enabled {
            "●".green().to_string()
        } else {
            "○".dimmed().to_string()
        };

        table.add_row(vec![
            format!("  {enabled_dot}"),
            id.bold().to_string(),
            display.to_string(),
            format!("key: {key_status}"),
        ]);
    }

    println!("{table}");
    println!();
    Ok(())
}

async fn set_key(
    client: &ClotoClient,
    provider: &str,
    key: Option<String>,
    json_mode: bool,
) -> Result<()> {
    let api_key = if let Some(k) = key {
        k
    } else {
        if json_mode {
            anyhow::bail!("--key is required in JSON mode");
        }
        Password::with_theme(&ColorfulTheme::default())
            .with_prompt(format!("  API key for {provider}"))
            .interact()?
    };

    let sp = if json_mode {
        None
    } else {
        Some(output::spinner("Setting API key..."))
    };

    let result: serde_json::Value = client.set_llm_provider_key(provider, &api_key).await?;

    if let Some(sp) = sp {
        sp.finish_and_clear();
    }

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    println!(
        "  {} API key set for {}",
        "✓".green().bold(),
        provider.bold()
    );
    println!();
    Ok(())
}

async fn delete_key(
    client: &ClotoClient,
    provider: &str,
    force: bool,
    json_mode: bool,
) -> Result<()> {
    if !force && !json_mode {
        let confirmed = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt(format!("  Remove API key for {provider}?"))
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
        Some(output::spinner("Removing API key..."))
    };

    let result: serde_json::Value = client.delete_llm_provider_key(provider).await?;

    if let Some(sp) = sp {
        sp.finish_and_clear();
    }

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    println!(
        "  {} API key removed for {}",
        "✓".green().bold(),
        provider.bold()
    );
    println!();
    Ok(())
}
