use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

pub fn render(f: &mut Frame) {
    let area = f.area();

    // Center the help overlay
    let width = 44.min(area.width.saturating_sub(4));
    let height = 14.min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup);

    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Tab       ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw("Switch pane"),
        ]),
        Line::from(vec![
            Span::styled("  ↑/k ↓/j   ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw("Navigate list"),
        ]),
        Line::from(vec![
            Span::styled("  r         ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw("Force refresh"),
        ]),
        Line::from(vec![
            Span::styled("  q         ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw("Quit"),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+C    ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw("Force quit"),
        ]),
        Line::from(vec![
            Span::styled("  ?         ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw("Toggle help"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Press any key to close",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let block = Block::default()
        .title(" Help ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(text).block(block);
    f.render_widget(paragraph, popup);
}
