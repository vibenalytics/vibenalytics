#![allow(dead_code)]
use ratatui::prelude::*;
use ratatui::widgets::*;
use super::theme;

/// Compute a centered rect of given width and height within `area`.
pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

pub fn render_confirm(frame: &mut Frame, area: Rect, title: &str, message: &str, confirm_focused: bool) {
    let width = 42.min(area.width);
    let height = 8.min(area.height);
    let rect = centered_rect(width, height, area);

    // Clear background
    frame.render_widget(Clear, rect);

    let confirm_style = if confirm_focused { theme::accent_bold() } else { theme::dim() };
    let cancel_style = if !confirm_focused { theme::accent_bold() } else { theme::dim() };

    let content = vec![
        Line::from(Span::styled(title, theme::accent_bold())),
        Line::from(""),
        Line::from(Span::styled(message, theme::text())),
        Line::from(""),
        Line::from(vec![
            Span::styled("    [ Confirm ]", confirm_style),
            Span::raw("    "),
            Span::styled("[ Cancel ]", cancel_style),
        ]),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::accent());
    let paragraph = Paragraph::new(content)
        .block(block)
        .alignment(Alignment::Center);
    frame.render_widget(paragraph, rect);
}

pub fn render_progress(frame: &mut Frame, area: Rect, title: &str, progress: f64, detail: &str) {
    let width = 42.min(area.width);
    let height = 7.min(area.height);
    let rect = centered_rect(width, height, area);

    frame.render_widget(Clear, rect);

    let bar_width = (width - 4) as usize;
    let filled = ((progress * bar_width as f64) as usize).min(bar_width);
    let empty = bar_width - filled;
    let bar = format!("[{}{}] {}%", "█".repeat(filled), "░".repeat(empty), (progress * 100.0) as u32);

    let content = vec![
        Line::from(Span::styled(title, theme::accent_bold())),
        Line::from(""),
        Line::from(Span::styled(bar, theme::accent())),
        Line::from(Span::styled(detail, theme::dim())),
        Line::from(Span::styled("Press Esc to cancel", theme::dim())),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::accent());
    let paragraph = Paragraph::new(content)
        .block(block)
        .alignment(Alignment::Center);
    frame.render_widget(paragraph, rect);
}
