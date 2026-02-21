use ratatui::prelude::*;
use ratatui::widgets::*;
use super::theme;

pub const TAB_NAMES: &[&str] = &["Overview", "Sessions", "Projects", "Settings"];

pub fn render_header(frame: &mut Frame, area: Rect, user_name: &str, connected: bool) {
    let status_style = if connected { theme::success() } else { theme::dim() };
    let dot = if connected { "●" } else { "○" };

    let line = Line::from(vec![
        Span::styled("  vibenalytics", theme::accent_bold()),
        Span::styled("  ", theme::dim()),
        Span::styled(dot, status_style),
        Span::styled(format!(" {user_name}"), theme::dim()),
    ]);

    frame.render_widget(Paragraph::new(line), area);
}

pub fn render_tab_bar(frame: &mut Frame, area: Rect, active_tab: usize) {
    let mut spans = vec![Span::raw("  ")];
    for (i, name) in TAB_NAMES.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", theme::dim()));
        }
        if i == active_tab {
            spans.push(Span::styled(*name, theme::accent_bold()));
        } else {
            spans.push(Span::styled(*name, theme::dim()));
        }
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

pub fn render_footer(frame: &mut Frame, area: Rect, hints: &str) {
    let line = Line::from(vec![
        Span::styled("  ", theme::dim()),
        Span::styled(hints, theme::dim()),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}
