use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::tui::app::App;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .title(" Metrics ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let content = if let Some(ref metrics) = app.metrics {
        let requests = metrics
            .get("total_requests")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let memories = metrics
            .get("total_memories")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let episodes = metrics
            .get("total_episodes")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let event_count = app.events.len();

        Line::from(vec![
            Span::styled("  Requests: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{requests}"),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  │  Memories: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{memories}"),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  │  Episodes: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{episodes}"),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  │  Events: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{event_count}"),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ])
    } else {
        Line::from(Span::styled(
            "  Connecting...",
            Style::default().fg(Color::DarkGray),
        ))
    };

    let paragraph = Paragraph::new(content).block(block);
    f.render_widget(paragraph, area);
}
