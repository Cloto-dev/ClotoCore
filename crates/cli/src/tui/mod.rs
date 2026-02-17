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

/// Restore terminal state (raw mode, alternate screen, cursor).
/// Called on both normal exit and panic to prevent terminal corruption.
fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) {
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();
}

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

    // bug-024: Install panic hook to restore terminal on panic
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(info);
    }));

    // bug-031: Larger channel to prevent startup deadlock with large history
    let (tx, mut rx) = mpsc::channel::<AppAction>(512);

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
            if let Ok(response) = sse_client.sse_stream().await {
                let mut stream = response.bytes_stream();
                let mut buffer = String::new();

                while let Some(chunk) = stream.next().await {
                    let Ok(chunk) = chunk else { break };
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
            // Reconnect after a delay
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    });

    // Initial fetch — use try_send to avoid blocking if channel fills (bug-031)
    if let Ok(agents) = client.get_agents().await {
        let _ = tx.try_send(AppAction::AgentsUpdated(agents));
    }
    if let Ok(history) = client.get_history().await {
        for event in history {
            let _ = tx.try_send(AppAction::NewEvent(event));
        }
    }

    let mut app = App::new(endpoint);

    // bug-024: Main loop with guaranteed cleanup on error
    let result = run_main_loop(&mut terminal, &mut app, &mut rx).await;

    // Restore terminal — always runs regardless of error
    restore_terminal(&mut terminal);

    result
}

/// Inner main loop, separated to guarantee cleanup runs even on `?` errors.
async fn run_main_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    rx: &mut mpsc::Receiver<AppAction>,
) -> Result<()> {
    loop {
        terminal.draw(|f| ui::draw(f, app))?;

        while let Ok(action) = rx.try_recv() {
            app.apply(action);
        }

        if !event::handle_events(app)? {
            break;
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}
