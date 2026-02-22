use ratatui::prelude::*;
use super::theme;

pub const ACTION_COUNT: usize = 4;

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
        if self.selected + 1 < ACTION_COUNT {
            self.selected += 1;
        }
    }
}

pub fn render(frame: &mut Frame, area: Rect, state: &SettingsState, user_name: &str, connected: bool, pending_events: usize, default_enabled: bool) {
    if !connected {
        let lines = vec![
            Line::from(""),
            Line::from(""),
            Line::from(Span::styled("  You are not logged in.", theme::dim())),
            Line::from(""),
            Line::from(vec![
                Span::styled("  > ", theme::accent_bold()),
                Span::styled("Login", theme::accent_bold()),
            ]),
        ];
        frame.render_widget(ratatui::widgets::Paragraph::new(lines), area);
        return;
    }

    let sync_mode = if default_enabled { "auto (all projects)" } else { "manual (whitelist)" };

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Account    ", theme::dim()),
            Span::styled(format!("● {user_name}"), theme::success()),
        ]),
        Line::from(vec![
            Span::styled("  Sync mode  ", theme::dim()),
            Span::styled(sync_mode, theme::text()),
        ]),
        Line::from(vec![
            Span::styled("  Pending    ", theme::dim()),
            Span::styled(format!("{pending_events} events"), theme::text()),
        ]),
        Line::from(""),
    ];

    let actions = [
        "Force Sync",
        "Import History",
        if default_enabled { "Switch to manual mode" } else { "Switch to auto mode" },
        "Logout",
    ];

    for (i, action) in actions.iter().enumerate() {
        let is_logout = i == actions.len() - 1;
        if is_logout {
            lines.push(Line::from(""));
        }
        let (marker, style) = if i == state.selected {
            if is_logout {
                ("> ", Style::default().fg(theme::ERROR).add_modifier(ratatui::style::Modifier::BOLD))
            } else {
                ("> ", theme::accent_bold())
            }
        } else if is_logout {
            ("  ", Style::default().fg(theme::ERROR))
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
