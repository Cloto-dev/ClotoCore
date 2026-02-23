use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};

use crate::tui::app::App;

pub fn render(f: &mut Frame, area: Rect, app: &App, is_active: bool) {
    let border_style = if is_active {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" Agents ")
        .borders(Borders::ALL)
        .border_style(border_style);

    if app.agents.is_empty() {
        let items = vec![ListItem::new(Span::styled(
            "  No agents registered",
            Style::default().fg(Color::DarkGray),
        ))];
        let list = List::new(items).block(block);
        f.render_widget(list, area);
        return;
    }

    let items: Vec<ListItem> = app
        .agents
        .iter()
        .map(|agent| {
            let dot = match agent.status.as_str() {
                "online" => Span::styled("● ", Style::default().fg(Color::Green)),
                "degraded" => Span::styled("◐ ", Style::default().fg(Color::Yellow)),
                _ => Span::styled("○ ", Style::default().fg(Color::DarkGray)),
            };

            let name = Span::styled(
                format!("{:<20}", agent.id),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            );

            let agent_type = if agent
                .default_engine_id
                .as_deref()
                .is_some_and(|e| e.starts_with("mind."))
                || agent.metadata.get("agent_type").is_some_and(|t| t == "ai")
            {
                Span::styled("AI     ", Style::default().fg(Color::Cyan))
            } else {
                Span::styled("Ctnr   ", Style::default().fg(Color::Magenta))
            };

            let status = Span::styled(
                format!("{:<10}", agent.status),
                Style::default().fg(Color::DarkGray),
            );

            ListItem::new(Line::from(vec![
                Span::raw("  "),
                dot,
                name,
                agent_type,
                status,
            ]))
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(app.agent_scroll));

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    f.render_stateful_widget(list, area, &mut state);
}
