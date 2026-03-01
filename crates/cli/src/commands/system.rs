use anyhow::Result;
use colored::Colorize;
use dialoguer::{theme::ColorfulTheme, Confirm};

use crate::cli::SystemCommand;
use crate::client::ClotoClient;
use crate::output;

pub async fn run(client: &ClotoClient, cmd: SystemCommand, json_mode: bool) -> Result<()> {
    match cmd {
        SystemCommand::Version => version(client, json_mode).await,
        SystemCommand::Health => health(client, json_mode).await,
        SystemCommand::Shutdown { force } => shutdown(client, force, json_mode).await,
        SystemCommand::InvalidateKey { force } => invalidate_key(client, force, json_mode).await,
        SystemCommand::Yolo { enable } => yolo(client, enable, json_mode).await,
    }
}

async fn version(client: &ClotoClient, json_mode: bool) -> Result<()> {
    let result: serde_json::Value = client.get_system_version().await?;

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    let version = result.get("version").and_then(|v| v.as_str()).unwrap_or("unknown");
    let target = result
        .get("build_target")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    output::print_header("Cloto Kernel");
    println!("  Version: {}", version.bold());
    println!("  Target:  {}", target.dimmed());
    println!();
    Ok(())
}

async fn health(client: &ClotoClient, json_mode: bool) -> Result<()> {
    let sp = if json_mode {
        None
    } else {
        Some(output::spinner("Checking health..."))
    };

    let result: serde_json::Value = client.get_system_health().await?;

    if let Some(sp) = sp {
        sp.finish_and_clear();
    }

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    let status = result.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");
    let icon = if status == "ok" {
        "●".green()
    } else {
        "●".red()
    };
    println!("  {} Kernel: {}", icon, status.bold());
    println!();
    Ok(())
}

async fn shutdown(client: &ClotoClient, force: bool, json_mode: bool) -> Result<()> {
    if !force && !json_mode {
        output::print_header("Shutdown Kernel");
        println!("  {}", "This will stop the Cloto kernel.".yellow());
        println!();
        let confirmed = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("  Confirm shutdown?")
            .default(false)
            .interact()?;
        if !confirmed {
            println!("  Cancelled.");
            return Ok(());
        }
    }

    let result: serde_json::Value = client.shutdown_system().await?;

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    println!("  {} Kernel is shutting down", "✓".green().bold());
    println!();
    Ok(())
}

async fn invalidate_key(client: &ClotoClient, force: bool, json_mode: bool) -> Result<()> {
    if !force && !json_mode {
        output::print_header("Invalidate API Key");
        println!(
            "  {}",
            "This will revoke the current API key permanently.".yellow()
        );
        println!();
        let confirmed = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("  Confirm key revocation?")
            .default(false)
            .interact()?;
        if !confirmed {
            println!("  Cancelled.");
            return Ok(());
        }
    }

    let result: serde_json::Value = client.invalidate_api_key().await?;

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    println!("  {} API key has been revoked", "✓".green().bold());
    println!();
    Ok(())
}

async fn yolo(client: &ClotoClient, enable: Option<bool>, json_mode: bool) -> Result<()> {
    if let Some(enabled) = enable {
        let result: serde_json::Value = client.set_yolo_mode(enabled).await?;

        if json_mode {
            println!("{}", serde_json::to_string_pretty(&result)?);
            return Ok(());
        }

        if enabled {
            println!(
                "  ⚡ YOLO mode {} — permission prompts disabled",
                "ON".yellow().bold()
            );
        } else {
            println!(
                "  {} YOLO mode {} — permission prompts enabled",
                "✓".green().bold(),
                "OFF".green().bold()
            );
        }
        println!();
        Ok(())
    } else {
        let result: serde_json::Value = client.get_yolo_mode().await?;

        if json_mode {
            println!("{}", serde_json::to_string_pretty(&result)?);
            return Ok(());
        }

        let enabled = result
            .get("enabled")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        output::print_header("YOLO Mode");
        if enabled {
            println!(
                "  Status: {} (permission prompts disabled)",
                "ON".yellow().bold()
            );
        } else {
            println!(
                "  Status: {} (permission prompts enabled)",
                "OFF".green().bold()
            );
        }
        println!();
        Ok(())
    }
}
