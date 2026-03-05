use anyhow::Result;
use colored::Colorize;
use comfy_table::presets::NOTHING;
use comfy_table::{ContentArrangement, Table};
use dialoguer::{theme::ColorfulTheme, Confirm};

use crate::cli::CronCommand;
use crate::client::ClotoClient;
use crate::output;

pub async fn run(client: &ClotoClient, cmd: CronCommand, json_mode: bool) -> Result<()> {
    match cmd {
        CronCommand::List { agent_id } => list(client, agent_id.as_deref(), json_mode).await,
        CronCommand::Create {
            agent_id,
            name,
            schedule_type,
            schedule_value,
            message,
            engine_id,
            max_iterations,
            hide_prompt,
        } => {
            create(
                client,
                &agent_id,
                &name,
                &schedule_type,
                &schedule_value,
                &message,
                engine_id.as_deref(),
                max_iterations,
                hide_prompt,
                json_mode,
            )
            .await
        }
        CronCommand::Delete { id, force } => delete(client, &id, force, json_mode).await,
        CronCommand::Toggle {
            id,
            enable,
            disable,
        } => {
            let enabled = if enable {
                true
            } else if disable {
                false
            } else {
                anyhow::bail!("Specify --enable or --disable");
            };
            toggle(client, &id, enabled, json_mode).await
        }
        CronCommand::Run { id } => run_now(client, &id, json_mode).await,
    }
}

async fn list(client: &ClotoClient, agent_id: Option<&str>, json_mode: bool) -> Result<()> {
    let sp = if json_mode {
        None
    } else {
        Some(output::spinner("Loading cron jobs..."))
    };

    let result: serde_json::Value = client.list_cron_jobs(agent_id).await?;

    if let Some(sp) = sp {
        sp.finish_and_clear();
    }

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    output::print_header("Cron Jobs");

    let jobs = result
        .get("jobs")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if jobs.is_empty() {
        println!("  {}", "No cron jobs configured.".dimmed());
        println!();
        return Ok(());
    }

    let mut table = Table::new();
    table
        .load_preset(NOTHING)
        .set_content_arrangement(ContentArrangement::Dynamic);

    for job in &jobs {
        let id = job.get("id").and_then(|v| v.as_str()).unwrap_or("-");
        let name = job.get("name").and_then(|v| v.as_str()).unwrap_or("-");
        let agent = job.get("agent_id").and_then(|v| v.as_str()).unwrap_or("-");
        let enabled = job
            .get("enabled")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let sched_type = job
            .get("schedule_type")
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        let sched_val = job
            .get("schedule_value")
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        let next_run = job
            .get("next_run_at")
            .and_then(|v| v.as_str())
            .unwrap_or("-");

        let generation = job
            .get("cron_generation")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        let dot = if enabled {
            "●".green().to_string()
        } else {
            "○".dimmed().to_string()
        };

        let schedule = format!("{sched_type}:{sched_val}");
        let gen_str = if generation > 0 {
            format!("[gen:{}]", generation).yellow().to_string()
        } else {
            String::new()
        };

        table.add_row(vec![
            format!("  {dot}"),
            id.bold().to_string(),
            format!("{name} {gen_str}"),
            agent.dimmed().to_string(),
            schedule,
            format!("next: {}", next_run.dimmed()),
        ]);
    }

    println!("{table}");
    println!();
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn create(
    client: &ClotoClient,
    agent_id: &str,
    name: &str,
    schedule_type: &str,
    schedule_value: &str,
    message: &str,
    engine_id: Option<&str>,
    max_iterations: Option<u32>,
    hide_prompt: bool,
    json_mode: bool,
) -> Result<()> {
    let mut body = serde_json::json!({
        "agent_id": agent_id,
        "name": name,
        "schedule_type": schedule_type,
        "schedule_value": schedule_value,
        "message": message,
    });

    if let Some(engine) = engine_id {
        body["engine_id"] = serde_json::json!(engine);
    }
    if let Some(max) = max_iterations {
        body["max_iterations"] = serde_json::json!(max);
    }
    if hide_prompt {
        body["hide_prompt"] = serde_json::json!(true);
    }

    let sp = if json_mode {
        None
    } else {
        Some(output::spinner("Creating cron job..."))
    };

    let result: serde_json::Value = client.create_cron_job(&body).await?;

    if let Some(sp) = sp {
        sp.finish_and_clear();
    }

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    let id = result
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let next = result
        .get("next_run_at")
        .and_then(|v| v.as_str())
        .unwrap_or("-");

    println!("  {} Cron job created: {}", "✓".green().bold(), id.bold());
    println!("  Next run: {}", next.dimmed());
    println!();
    Ok(())
}

async fn delete(client: &ClotoClient, id: &str, force: bool, json_mode: bool) -> Result<()> {
    if !force && !json_mode {
        let confirmed = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt(format!("  Delete cron job {id}?"))
            .default(false)
            .interact()?;
        if !confirmed {
            println!("  Cancelled.");
            return Ok(());
        }
    }

    let result: serde_json::Value = client.delete_cron_job(id).await?;

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    println!("  {} Cron job deleted: {}", "✓".green().bold(), id.bold());
    println!();
    Ok(())
}

async fn toggle(client: &ClotoClient, id: &str, enabled: bool, json_mode: bool) -> Result<()> {
    let result: serde_json::Value = client.toggle_cron_job(id, enabled).await?;

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    let state = if enabled {
        "enabled".green().bold()
    } else {
        "disabled".red().bold()
    };
    println!("  {} Cron job {}: {}", "✓".green().bold(), id.bold(), state);
    println!();
    Ok(())
}

async fn run_now(client: &ClotoClient, id: &str, json_mode: bool) -> Result<()> {
    let sp = if json_mode {
        None
    } else {
        Some(output::spinner("Dispatching cron job..."))
    };

    let result: serde_json::Value = client.run_cron_job(id).await?;

    if let Some(sp) = sp {
        sp.finish_and_clear();
    }

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    println!(
        "  {} Cron job dispatched: {}",
        "✓".green().bold(),
        id.bold()
    );
    println!();
    Ok(())
}
