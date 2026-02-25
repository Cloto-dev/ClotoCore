use anyhow::Result;
use colored::Colorize;

use crate::cli::ConfigCommand;
use crate::config::CliConfig;

pub fn run(cmd: ConfigCommand, config: &CliConfig) -> Result<()> {
    match cmd {
        ConfigCommand::Show => show(config),
        ConfigCommand::Set { key, value } => set(&key, &value),
    }
}

fn show(config: &CliConfig) -> Result<()> {
    let path = CliConfig::path()?;

    println!();
    println!("  {}", "Configuration".bold());
    println!("  {}", "─".repeat(36).dimmed());
    println!("  {}   {}", "file:".dimmed(), path.display());
    println!("  {}    {}", "url:".dimmed(), config.url);
    println!(
        "  {}",
        format!(
            "api_key: {}",
            match &config.api_key {
                Some(k) if k.len() > 8 => format!("{}...{}", &k[..4], &k[k.len() - 4..]),
                Some(_) => "***".to_string(),
                None => "(not set)".dimmed().to_string(),
            }
        )
        .dimmed()
    );
    println!();

    // Show environment overrides if active
    if std::env::var("CLOTO_URL").is_ok() {
        println!("  {} CLOTO_URL environment variable is active", "ℹ".blue());
    }
    if std::env::var("CLOTO_API_KEY").is_ok() {
        println!(
            "  {} CLOTO_API_KEY environment variable is active",
            "ℹ".blue()
        );
    }

    Ok(())
}

fn set(key: &str, value: &str) -> Result<()> {
    CliConfig::set(key, value)?;

    println!(
        "  {} {key} = {}",
        "✓".green().bold(),
        if key == "api_key" {
            "***".to_string()
        } else {
            value.to_string()
        }
    );
    Ok(())
}
