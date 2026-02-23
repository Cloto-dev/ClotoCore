use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};

use crate::tui::app::App;

#[allow(clippy::too_many_lines)]
pub fn render(f: &mut Frame, area: Rect, app: &App, is_active: bool) {
    let border_style = if is_active {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" Events ")
        .borders(Borders::ALL)
        .border_style(border_style);

    if app.events.is_empty() {
        let items = vec![ListItem::new(Span::styled(
            "  Waiting for events...",
            Style::default().fg(Color::DarkGray),
        ))];
        let list = List::new(items).block(block);
        f.render_widget(list, area);
        return;
    }

    // Show events in reverse order (newest first)
    let items: Vec<ListItem> = app
        .events
        .iter()
        .rev()
        .map(|event| {
            let event_type = event
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("Unknown");

            let timestamp = event
                .get("timestamp")
                .and_then(|t| t.as_str())
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .map_or_else(
                    || "??:??:??".to_string(),
                    |dt| dt.format("%H:%M:%S").to_string(),
                );

            let data = event
                .get("data")
                .cloned()
                .unwrap_or(serde_json::Value::Null);

            let (type_color, detail) = match event_type {
                "MessageReceived" => {
                    let source = data
                        .get("source")
                        .and_then(|s| s.get("name").or(s.get("id")))
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    (Color::Cyan, format!("{source} → message"))
                }
                "ThoughtRequested" => {
                    let engine = data
                        .get("engine_id")
                        .and_then(|e| e.as_str())
                        .unwrap_or("?");
                    (Color::Green, format!("{engine} thinking"))
                }
                "ThoughtResponse" => {
                    let agent = data.get("agent_id").and_then(|a| a.as_str()).unwrap_or("?");
                    (Color::Green, format!("{agent} responded"))
                }
                "AgentPowerChanged" => {
                    let agent = data.get("agent_id").and_then(|a| a.as_str()).unwrap_or("?");
                    let on = data
                        .get("enabled")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    (
                        Color::Magenta,
                        format!("{agent} {}", if on { "ON" } else { "OFF" }),
                    )
                }
                "SystemNotification" => (Color::Yellow, "System notification".to_string()),
                "ConfigUpdated" => {
                    let plugin = data
                        .get("plugin_id")
                        .and_then(|p| p.as_str())
                        .unwrap_or("?");
                    (Color::Yellow, format!("{plugin} config"))
                }
                _ => (Color::DarkGray, event_type.to_string()),
            };

            let time_span = Span::styled(
                format!("  {timestamp} "),
                Style::default().fg(Color::DarkGray),
            );
            let type_span = Span::styled(
                format!("[{:<18}] ", event_type),
                Style::default().fg(type_color),
            );
            let detail_span = Span::styled(detail, Style::default().fg(Color::White));

            ListItem::new(Line::from(vec![time_span, type_span, detail_span]))
        })
        .collect();

    let mut state = ListState::default();
    if !app.events.is_empty() {
        // bug-029: Convert logical index to display index.
        // The list is rendered in reverse order, so display_index = len - 1 - logical_index.
        let display_index = app.events.len() - 1 - app.event_scroll.min(app.events.len() - 1);
        state.select(Some(display_index));
    }

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().bg(Color::DarkGray));

    f.render_stateful_widget(list, area, &mut state);
}
