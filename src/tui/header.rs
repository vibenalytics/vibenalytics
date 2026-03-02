use ratatui::prelude::*;
use ratatui::widgets::*;
use super::theme;

pub const TAB_NAMES: &[&str] = &["Overview", "Projects", "Settings"];

pub fn render_header(frame: &mut Frame, area: Rect, user_name: &str, connected: bool) {
    // Split into: blank line | header content
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),  // top margin
            Constraint::Length(1),  // header text
        ])
        .split(area);

    let status_style = if connected { theme::success() } else { theme::dim() };
    let dot = if connected { "●" } else { "○" };

    let line = Line::from(vec![
        Span::styled("  vibenalytics", theme::accent_bold()),
        Span::styled("  ", theme::dim()),
        Span::styled(dot, status_style),
        Span::styled(format!(" {user_name}"), theme::dim()),
    ]);

    frame.render_widget(Paragraph::new(line), rows[1]);
}

pub fn render_tab_bar(frame: &mut Frame, area: Rect, active_tab: usize) {
    // Split into: tab labels | underline
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // tab labels
            Constraint::Length(1),  // underline
        ])
        .split(area);

    let mut spans = vec![Span::raw("  ")];
    for (i, name) in TAB_NAMES.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("   ", theme::dim()));
        }
        if i == active_tab {
            spans.push(Span::styled(*name, theme::accent_bold()));
        } else {
            spans.push(Span::styled(*name, theme::dim()));
        }
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), rows[0]);

    // Build underline: accent marks under active tab, dim line elsewhere
    let mut ul_spans: Vec<Span> = Vec::new();
    ul_spans.push(Span::styled("  ", Style::default().fg(theme::BORDER)));
    for (i, name) in TAB_NAMES.iter().enumerate() {
        if i > 0 {
            ul_spans.push(Span::styled("───", Style::default().fg(theme::BORDER)));
        }
        let bar = "─".repeat(name.len());
        if i == active_tab {
            ul_spans.push(Span::styled(bar, theme::accent_bold()));
        } else {
            ul_spans.push(Span::styled(bar, Style::default().fg(theme::BORDER)));
        }
    }
    // Fill rest with dim line
    let used: usize = 2 + TAB_NAMES.iter().map(|n| n.len()).sum::<usize>() + (TAB_NAMES.len() - 1) * 3;
    let remaining = area.width as usize - used.min(area.width as usize);
    if remaining > 0 {
        ul_spans.push(Span::styled("─".repeat(remaining), Style::default().fg(theme::BORDER)));
    }

    frame.render_widget(Paragraph::new(Line::from(ul_spans)), rows[1]);
}

pub fn render_footer(frame: &mut Frame, area: Rect, hints: &str) {
    // Split into: separator | hint text
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // separator
            Constraint::Length(1),  // hints
        ])
        .split(area);

    let sep = "─".repeat(area.width as usize);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(sep, Style::default().fg(theme::BORDER)))),
        rows[0],
    );

    let line = Line::from(vec![
        Span::styled("  ", theme::dim()),
        Span::styled(hints, theme::dim()),
    ]);
    frame.render_widget(Paragraph::new(line), rows[1]);
}
