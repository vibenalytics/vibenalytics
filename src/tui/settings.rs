use ratatui::prelude::*;
use super::theme;

pub const ACTIONS: &[&str] = &["Re-authenticate", "Force Sync", "Import History"];

pub struct SettingsState {
    pub selected: usize,
}

impl Default for SettingsState {
    fn default() -> Self {
        SettingsState { selected: 0 }
    }
}

impl SettingsState {
    pub fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }
    pub fn down(&mut self) {
        if self.selected + 1 < ACTIONS.len() {
            self.selected += 1;
        }
    }
}

pub fn render(frame: &mut Frame, area: Rect, state: &SettingsState, user_name: &str, connected: bool, pending_events: usize) {
    let status_style = if connected { theme::success() } else { theme::dim() };
    let dot = if connected { "●" } else { "○" };

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Account  ", theme::dim()),
            Span::styled(dot, status_style),
            Span::styled(format!(" {user_name}"), theme::text()),
        ]),
        Line::from(vec![
            Span::styled("  Pending  ", theme::dim()),
            Span::styled(format!("{pending_events} events"), theme::text()),
        ]),
        Line::from(""),
    ];

    for (i, action) in ACTIONS.iter().enumerate() {
        let (marker, style) = if i == state.selected {
            ("> ", theme::accent_bold())
        } else {
            ("  ", theme::dim())
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {marker}"), style),
            Span::styled(*action, style),
        ]));
    }

    frame.render_widget(ratatui::widgets::Paragraph::new(lines), area);
}
