use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use super::app::{App, Pane};
use super::widgets;

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();

    // Main layout: Header | Content | Metrics | Footer
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(8),    // Content (agents + events)
            Constraint::Length(3), // Metrics
            Constraint::Length(1), // Footer
        ])
        .split(area);

    // Header
    render_header(f, main_chunks[0], app);

    // Content: Agents | Events (side by side)
    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(main_chunks[1]);

    widgets::agents::render(f, content_chunks[0], app, app.active_pane == Pane::Agents);
    widgets::events::render(f, content_chunks[1], app, app.active_pane == Pane::Events);

    // Metrics
    widgets::metrics::render(f, main_chunks[2], app);

    // Footer
    render_footer(f, main_chunks[3], app);

    // Help overlay
    if app.show_help {
        widgets::help::render(f);
    }
}

fn render_header(f: &mut Frame, area: Rect, app: &App) {
    let status_dot = if app.connected { "●" } else { "○" };
    let status_color = if app.connected {
        Color::Green
    } else {
        Color::Red
    };

    let header = Line::from(vec![
        Span::styled(
            "  Cloto Dashboard",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  v{}", env!("CARGO_PKG_VERSION")),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw("    "),
        Span::styled(status_dot, Style::default().fg(status_color)),
        Span::styled(
            format!("  {}", app.endpoint),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(Color::DarkGray));

    let paragraph = Paragraph::new(header).block(block);
    f.render_widget(paragraph, area);
}

fn render_footer(f: &mut Frame, area: Rect, _app: &App) {
    let footer = Line::from(vec![
        Span::styled("  [Tab]", Style::default().fg(Color::Cyan)),
        Span::styled(" Pane  ", Style::default().fg(Color::DarkGray)),
        Span::styled("[↑↓]", Style::default().fg(Color::Cyan)),
        Span::styled(" Navigate  ", Style::default().fg(Color::DarkGray)),
        Span::styled("[r]", Style::default().fg(Color::Cyan)),
        Span::styled(" Refresh  ", Style::default().fg(Color::DarkGray)),
        Span::styled("[q]", Style::default().fg(Color::Cyan)),
        Span::styled(" Quit  ", Style::default().fg(Color::DarkGray)),
        Span::styled("[?]", Style::default().fg(Color::Cyan)),
        Span::styled(" Help", Style::default().fg(Color::DarkGray)),
    ]);

    let paragraph = Paragraph::new(footer);
    f.render_widget(paragraph, area);
}
