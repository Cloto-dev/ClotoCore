use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use std::time::Duration;

use super::app::App;

/// Poll for keyboard events with a timeout.
/// Returns true if the app should continue running.
pub fn handle_events(app: &mut App) -> anyhow::Result<bool> {
    if event::poll(Duration::from_millis(100))? {
        if let Event::Key(key) = event::read()? {
            // Ctrl+C always quits
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                app.should_quit = true;
                return Ok(false);
            }

            // Help overlay intercepts all keys
            if app.show_help {
                app.show_help = false;
                return Ok(true);
            }

            match key.code {
                KeyCode::Char('q') => {
                    app.should_quit = true;
                    return Ok(false);
                }
                KeyCode::Char('?') => {
                    app.show_help = !app.show_help;
                }
                KeyCode::Tab | KeyCode::BackTab => {
                    app.active_pane = app.active_pane.next();
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    app.scroll_up();
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    app.scroll_down();
                }
                KeyCode::Char('r') => {
                    // Force refresh handled in main loop via flag
                    app.last_refresh =
                        std::time::Instant::now() - std::time::Duration::from_secs(60);
                }
                _ => {}
            }
        }
    }
    Ok(true)
}
