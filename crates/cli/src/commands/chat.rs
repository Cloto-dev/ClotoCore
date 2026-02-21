use anyhow::{Context, Result};
use colored::Colorize;
use futures::StreamExt;

use crate::client::ExivClient;
use crate::output;

pub async fn run(client: &ExivClient, agent: &str, message: &str, json_mode: bool) -> Result<()> {
    if !json_mode {
        println!("  {}: {message}", "You".bold());
    }

    // Build ExivMessage
    let msg = exiv_shared::ExivMessage {
        id: exiv_shared::ExivId::new().to_string(),
        source: exiv_shared::MessageSource::User {
            id: "cli-user".to_string(),
            name: "CLI".to_string(),
        },
        target_agent: Some(agent.to_string()),
        content: message.to_string(),
        timestamp: chrono::Utc::now(),
        metadata: std::collections::HashMap::new(),
    };

    // Send chat message
    client
        .send_chat(&msg)
        .await
        .context("Failed to send message")?;

    // Connect to SSE stream and wait for ThoughtResponse
    let sp = if !json_mode {
        Some(output::spinner("Thinking..."))
    } else {
        None
    };

    let response = client
        .sse_stream()
        .await
        .context("Failed to connect to event stream")?;

    let start = std::time::Instant::now();
    let timeout_duration = std::time::Duration::from_secs(60);

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut reply = None;

    // bug-030: Use tokio::time::timeout to enforce deadline on each chunk read,
    // preventing indefinite blocking when server goes silent after keep-alive.
    loop {
        let chunk_result = tokio::time::timeout(timeout_duration, stream.next()).await;

        match chunk_result {
            Err(_) => {
                // Timeout elapsed waiting for next chunk
                if let Some(ref sp) = sp {
                    sp.finish_and_clear();
                }
                anyhow::bail!("Timeout: no response received within 60 seconds");
            }
            Ok(None) => {
                // Stream ended
                break;
            }
            Ok(Some(chunk)) => {
                let chunk = chunk.context("Stream read error")?;
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                // Process complete SSE lines
                while let Some(pos) = buffer.find("\n\n") {
                    let event_block = buffer[..pos].to_string();
                    buffer = buffer[pos + 2..].to_string();

                    for line in event_block.lines() {
                        if let Some(data) = line.strip_prefix("data:") {
                            let data = data.trim();
                            if data == "connected" || data == "keep-alive" {
                                continue;
                            }

                            if let Ok(event) = serde_json::from_str::<serde_json::Value>(data) {
                                if let Some(event_data) = event.get("data") {
                                    let event_type =
                                        event_data.get("type").and_then(|t| t.as_str());

                                    if event_type == Some("ThoughtResponse") {
                                        if let Some(inner) = event_data.get("data") {
                                            let resp_agent = inner
                                                .get("agent_id")
                                                .and_then(|a| a.as_str())
                                                .unwrap_or("");

                                            if resp_agent == agent {
                                                let content = inner
                                                    .get("content")
                                                    .and_then(|c| c.as_str())
                                                    .unwrap_or("")
                                                    .to_string();

                                                let engine = inner
                                                    .get("engine_id")
                                                    .and_then(|e| e.as_str())
                                                    .unwrap_or("unknown")
                                                    .to_string();

                                                reply = Some((content, engine));
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if reply.is_some() {
                        break;
                    }
                }

                if reply.is_some() {
                    break;
                }

                // Check overall elapsed time
                if start.elapsed() > timeout_duration {
                    if let Some(ref sp) = sp {
                        sp.finish_and_clear();
                    }
                    anyhow::bail!("Timeout: no response received within 60 seconds");
                }
            }
        }
    }

    if let Some(sp) = sp {
        sp.finish_and_clear();
    }

    match reply {
        Some((content, engine)) => {
            let latency_ms = start.elapsed().as_millis();

            if json_mode {
                let data = serde_json::json!({
                    "agent": agent,
                    "input": message,
                    "response": content,
                    "engine": engine,
                    "latency_ms": latency_ms,
                });
                println!("{}", serde_json::to_string_pretty(&data)?);
            } else {
                println!("  {}: {content}", agent.cyan().bold());
            }
        }
        None => {
            if json_mode {
                let data = serde_json::json!({
                    "agent": agent,
                    "input": message,
                    "response": null,
                    "error": "No response received",
                });
                println!("{}", serde_json::to_string_pretty(&data)?);
            } else {
                println!(
                    "  {} No response received from {agent}",
                    "!".yellow().bold()
                );
            }
        }
    }

    Ok(())
}
