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
        Some(output::spinner("Loading memories..."))
    };

    let result: serde_json::Value = client.get_memories().await?;

    if let Some(sp) = sp {
        sp.finish_and_clear();
    }

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    output::print_header("Memories");

    let memories = result
        .get("memories")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let count = result
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);

    if memories.is_empty() {
        println!("  {}", "No memories stored.".dimmed());
        println!();
        return Ok(());
    }

    let mut table = Table::new();
    table
        .load_preset(NOTHING)
        .set_content_arrangement(ContentArrangement::Dynamic);

    for mem in &memories {
        let id = mem
            .get("id")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        let content = mem.get("content").and_then(|v| v.as_str()).unwrap_or("-");
        let truncated = if content.len() > 80 {
            format!("{}...", &content[..77])
        } else {
            content.to_string()
        };
        let created = mem
            .get("created_at")
            .and_then(|v| v.as_str())
            .unwrap_or("-");

        table.add_row(vec![
            format!("  {id}"),
            truncated,
            created.dimmed().to_string(),
        ]);
    }

    println!("{table}");
    println!();
    println!("  {} memories total", count.to_string().bold());
    println!();
    Ok(())
}
