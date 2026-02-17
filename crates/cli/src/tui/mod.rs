pub mod app;
pub mod event;
pub mod ui;
pub mod widgets;

use anyhow::{Context, Result};
use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::prelude::*;
use std::io;
use tokio::sync::mpsc;

use crate::client::ExivClient;
use crate::config::CliConfig;
use app::{App, AppAction};

/// Launch the TUI dashboard.
pub async fn run() -> Result<()> {
    let config = CliConfig::load()?;
    let client = ExivClient::new(&config);
    let endpoint = config.url.clone();

    // Setup terminal
    enable_raw_mode().context("Failed to enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("Failed to enter alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("Failed to create terminal")?;

    // Channel for background tasks to send updates
    let (tx, mut rx) = mpsc::channel::<AppAction>(64);

    // Spawn background polling task (agents, plugins, metrics)
    let poll_client = ExivClient::new(&config);
    let poll_tx = tx.clone();
    tokio::spawn(async move {
        loop {
            // Fetch agents
            if let Ok(agents) = poll_client.get_agents().await {
                let _ = poll_tx.send(AppAction::AgentsUpdated(agents)).await;
            }
            // Fetch plugins
            if let Ok(plugins) = poll_client.get_plugins().await {
                let _ = poll_tx.send(AppAction::PluginsUpdated(plugins)).await;
            }
            // Fetch metrics
            if let Ok(metrics) = poll_client.get_metrics().await {
                let _ = poll_tx.send(AppAction::MetricsUpdated(metrics)).await;
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });

    // Spawn SSE listener task
    let sse_client = ExivClient::new(&config);
    let sse_tx = tx.clone();
    tokio::spawn(async move {
        loop {
            match sse_client.sse_stream().await {
                Ok(response) => {
                    let mut stream = response.bytes_stream();
                    let mut buffer = String::new();

                    while let Some(chunk) = stream.next().await {
                        let chunk = match chunk {
                            Ok(c) => c,
                            Err(_) => break,
                        };
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
                                        let _ = sse_tx.send(AppAction::NewEvent(event)).await;
                                    }
                                }
                            }
                        }
                    }
                }
                Err(_) => {}
            }
            // Reconnect after a delay
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    });

    // Initial fetch
    if let Ok(agents) = client.get_agents().await {
        let _ = tx.send(AppAction::AgentsUpdated(agents)).await;
    }
    if let Ok(history) = client.get_history().await {
        for event in history {
            let _ = tx.send(AppAction::NewEvent(event)).await;
        }
    }

    let mut app = App::new(endpoint);

    // Main loop
    loop {
        // Draw
        terminal.draw(|f| ui::draw(f, &app))?;

        // Process pending actions (non-blocking drain)
        while let Ok(action) = rx.try_recv() {
            app.apply(action);
        }

        // Handle keyboard events
        if !event::handle_events(&mut app)? {
            break;
        }

        if app.should_quit {
            break;
        }
    }

    // Restore terminal
    disable_raw_mode().context("Failed to disable raw mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("Failed to leave alternate screen")?;
    terminal.show_cursor().context("Failed to show cursor")?;

    Ok(())
}
