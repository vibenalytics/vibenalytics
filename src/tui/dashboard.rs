use ratatui::prelude::*;
use ratatui::widgets::*;
use super::theme;

pub fn render(frame: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Today", theme::accent_bold()),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("    Sessions   ", theme::dim()),
            Span::styled("--", theme::text()),
        ]),
        Line::from(vec![
            Span::styled("    Prompts    ", theme::dim()),
            Span::styled("--", theme::text()),
        ]),
        Line::from(vec![
            Span::styled("    Tool calls ", theme::dim()),
            Span::styled("--", theme::text()),
        ]),
        Line::from(vec![
            Span::styled("    Time       ", theme::dim()),
            Span::styled("--", theme::text()),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  This week", theme::accent_bold()),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("    Sessions   ", theme::dim()),
            Span::styled("--", theme::text()),
        ]),
        Line::from(vec![
            Span::styled("    Prompts    ", theme::dim()),
            Span::styled("--", theme::text()),
        ]),
        Line::from(vec![
            Span::styled("    Tool calls ", theme::dim()),
            Span::styled("--", theme::text()),
        ]),
        Line::from(vec![
            Span::styled("    Time       ", theme::dim()),
            Span::styled("--", theme::text()),
        ]),
    ];

    frame.render_widget(Paragraph::new(lines), area);
}
