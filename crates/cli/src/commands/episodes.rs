use anyhow::Result;
use colored::Colorize;
use comfy_table::presets::NOTHING;
use comfy_table::{ContentArrangement, Table};

use crate::client::ClotoClient;
use crate::output;

pub async fn run(client: &ClotoClient, json_mode: bool) -> Result<()> {
    let sp = if json_mode {
        None
    } else {
        Some(output::spinner("Loading episodes..."))
    };

    let result: serde_json::Value = client.get_episodes().await?;

    if let Some(sp) = sp {
        sp.finish_and_clear();
    }

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    output::print_header("Episodes");

    let episodes = result
        .get("episodes")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let count = result
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);

    if episodes.is_empty() {
        println!("  {}", "No episodes archived.".dimmed());
        println!();
        return Ok(());
    }

    let mut table = Table::new();
    table
        .load_preset(NOTHING)
        .set_content_arrangement(ContentArrangement::Dynamic);

    for ep in &episodes {
        let id = ep
            .get("id")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        let summary = ep.get("summary").and_then(|v| v.as_str()).unwrap_or("-");
        let truncated = if summary.len() > 80 {
            format!("{}...", &summary[..77])
        } else {
            summary.to_string()
        };
        let created = ep.get("created_at").and_then(|v| v.as_str()).unwrap_or("-");

        table.add_row(vec![
            format!("  {id}"),
            truncated,
            created.dimmed().to_string(),
        ]);
    }

    println!("{table}");
    println!();
    println!("  {} episodes total", count.to_string().bold());
    println!();
    Ok(())
}
