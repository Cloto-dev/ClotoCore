use anyhow::{Context, Result};
use colored::Colorize;
use futures::StreamExt;

use crate::client::ExivClient;
use crate::output;

pub async fn run(client: &ExivClient, follow: bool, limit: usize, json_mode: bool) -> Result<()> {
    if follow {
        follow_stream(client, json_mode).await
    } else {
        show_history(client, limit, json_mode).await
    }
}

/// Display recent event history from the ring buffer.
async fn show_history(client: &ExivClient, limit: usize, json_mode: bool) -> Result<()> {
    let sp = if !json_mode { Some(output::spinner("Loading event history...")) } else { None };
    let history: Vec<serde_json::Value> = client.get_history().await?;
    if let Some(sp) = sp { sp.finish_and_clear(); }

    if json_mode {
        let limited: Vec<_> = history.iter().rev().take(limit).collect();
        println!("{}", serde_json::to_string_pretty(&limited)?);
        return Ok(());
    }

    if history.is_empty() {
        output::print_header("Event Log");
        println!("  {}", "No events recorded.".dimmed());
        println!();
        return Ok(());
    }

    output::print_header("Event Log");

    // History is oldest-first from API; show most recent first, limited
    let events: Vec<_> = history.iter().rev().take(limit).collect();
    for event in events.iter().rev() {
        print_event(event);
    }
    println!();

    let total = history.len();
    if total > limit {
        println!("  {} Showing {limit} of {total} events. Use {} to see more.",
            "ℹ".dimmed(),
            "--limit N".dimmed(),
        );
        println!();
    }

    Ok(())
}

/// Follow SSE stream and print events in real-time.
async fn follow_stream(client: &ExivClient, json_mode: bool) -> Result<()> {
    if !json_mode {
        output::print_header("Live Event Stream");
        println!("  {} Press {} to stop", "ℹ".dimmed(), "Ctrl+C".bold());
        println!();
    }

    let response = client.sse_stream().await
        .context("Failed to connect to event stream")?;

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("Stream read error")?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = buffer.find("\n\n") {
            let event_block = buffer[..pos].to_string();
            buffer = buffer[pos + 2..].to_string();

            for line in event_block.lines() {
                if let Some(data) = line.strip_prefix("data:") {
                    let data = data.trim();
                    if data == "connected" || data == "keep-alive" || data.is_empty() {
                        continue;
                    }

                    if let Ok(event) = serde_json::from_str::<serde_json::Value>(data) {
                        if json_mode {
                            println!("{}", serde_json::to_string(&event)?);
                        } else {
                            print_event(&event);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Format and print a single event with color coding.
fn print_event(event: &serde_json::Value) {
    let event_type = event.get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("Unknown");

    let timestamp = event.get("timestamp")
        .and_then(|t| t.as_str())
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|dt| dt.format("%H:%M:%S").to_string())
        .unwrap_or_else(|| "??:??:??".to_string());

    let data = event.get("data").cloned().unwrap_or(serde_json::Value::Null);

    let (tag, detail) = match event_type {
        "MessageReceived" => {
            let source = data.get("source")
                .and_then(|s| s.get("name").or(s.get("id")))
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let target = data.get("target_agent")
                .and_then(|t| t.as_str())
                .unwrap_or("system");
            let content = data.get("content")
                .and_then(|c| c.as_str())
                .unwrap_or("");
            let preview = if content.len() > 60 {
                format!("\"{}...\"", &content[..57])
            } else {
                format!("\"{content}\"")
            };
            (
                format!("[{}]", "MessageReceived".cyan()),
                format!("{source} → {target}: {preview}"),
            )
        }
        "ThoughtRequested" => {
            let engine = data.get("engine_id")
                .and_then(|e| e.as_str())
                .unwrap_or("?");
            let agent = data.get("agent")
                .and_then(|a| a.get("id"))
                .and_then(|id| id.as_str())
                .unwrap_or("?");
            (
                format!("[{}]", "ThoughtRequested".green()),
                format!("{engine} thinking for {agent}"),
            )
        }
        "ThoughtResponse" => {
            let agent = data.get("agent_id")
                .and_then(|a| a.as_str())
                .unwrap_or("?");
            let content = data.get("content")
                .and_then(|c| c.as_str())
                .unwrap_or("");
            let preview = if content.len() > 60 {
                format!("\"{}...\"", &content[..57])
            } else {
                format!("\"{content}\"")
            };
            (
                format!("[{}]", "ThoughtResponse".green().bold()),
                format!("{agent}: {preview}"),
            )
        }
        "AgentPowerChanged" => {
            let agent = data.get("agent_id")
                .and_then(|a| a.as_str())
                .unwrap_or("?");
            let enabled = data.get("enabled")
                .and_then(|e| e.as_bool())
                .unwrap_or(false);
            let state = if enabled { "ON".green().to_string() } else { "OFF".red().to_string() };
            (
                format!("[{}]", "PowerChanged".magenta()),
                format!("{agent} → {state}"),
            )
        }
        "SystemNotification" => {
            let msg = if data.is_string() {
                data.as_str().unwrap_or("").to_string()
            } else {
                data.to_string()
            };
            (
                format!("[{}]", "System".yellow()),
                msg,
            )
        }
        "ConfigUpdated" => {
            let plugin = data.get("plugin_id")
                .and_then(|p| p.as_str())
                .unwrap_or("?");
            (
                format!("[{}]", "ConfigUpdated".yellow()),
                format!("Plugin {plugin} config updated"),
            )
        }
        "PermissionGranted" => {
            let plugin = data.get("plugin_id")
                .and_then(|p| p.as_str())
                .unwrap_or("?");
            let perm = data.get("permission")
                .and_then(|p| p.as_str())
                .unwrap_or("?");
            (
                format!("[{}]", "PermGranted".blue()),
                format!("{plugin}: {perm}"),
            )
        }
        _ => {
            (
                format!("[{}]", event_type.dimmed()),
                format!("{}", serde_json::to_string(&data).unwrap_or_default()).dimmed().to_string(),
            )
        }
    };

    println!("  {} {:<28} {}", timestamp.dimmed(), tag, detail);
}
