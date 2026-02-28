use std::path::Path;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};
use super::theme;

pub const ACTION_COUNT: usize = 6;

pub struct SettingsState {
    pub selected: usize,
    pub debug_scroll: u16,
}

impl Default for SettingsState {
    fn default() -> Self {
        SettingsState { selected: 0, debug_scroll: 0 }
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
    pub fn scroll_debug_up(&mut self) {
        self.debug_scroll = self.debug_scroll.saturating_sub(3);
    }
    pub fn scroll_debug_down(&mut self, max_lines: usize, visible: u16) {
        let max_scroll = (max_lines as u16).saturating_sub(visible);
        if self.debug_scroll < max_scroll {
            self.debug_scroll = (self.debug_scroll + 3).min(max_scroll);
        }
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &SettingsState,
    user_name: &str,
    connected: bool,
    default_enabled: bool,
    auto_sync: bool,
    local_sync: bool,
    debug_mode: bool,
    debug_lines: &[String],
    data_dir: &Path,
) {
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
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }

    // Split: settings info+actions on top, debug log on bottom (when enabled)
    let settings_height = if debug_mode { 16u16 } else { 15u16 };
    let (settings_area, debug_area) = if debug_mode && !debug_lines.is_empty() {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(settings_height),
                Constraint::Min(3),
            ])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    let sync_mode = if default_enabled { "auto (all projects)" } else { "manual (whitelist)" };

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Account      ", theme::dim()),
            Span::styled(format!("● {user_name}"), theme::success()),
        ]),
        Line::from(vec![
            Span::styled("  Auto sync    ", theme::dim()),
            Span::styled(if auto_sync { "on" } else { "off" }, if auto_sync { theme::success() } else { theme::warning() }),
        ]),
        Line::from(vec![
            Span::styled("  Sync mode    ", theme::dim()),
            Span::styled(sync_mode, theme::text()),
        ]),
    ];
    if debug_mode {
        lines.push(Line::from(vec![
            Span::styled("  Debug dir    ", theme::dim()),
            Span::styled(data_dir.display().to_string(), theme::warning()),
        ]));
    }
    lines.push(Line::from(""));

    let auto_sync_label = if auto_sync { "Disable auto sync" } else { "Enable auto sync" };
    let local_sync_label = if local_sync { "[DEV] Local sync: ON" } else { "[DEV] Local sync: OFF" };
    let debug_label = if debug_mode { "Debug Mode: ON" } else { "Debug Mode: OFF" };
    let actions = [
        "Force Sync",
        "Import History",
        auto_sync_label,
        if default_enabled { "Switch to manual mode" } else { "Switch to auto mode" },
        local_sync_label,
        debug_label,
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

    frame.render_widget(Paragraph::new(lines), settings_area);

    // Debug log panel
    if let Some(log_area) = debug_area {
        let block = Block::default()
            .title(Span::styled(" Debug Log ", theme::accent()))
            .borders(Borders::TOP)
            .border_style(theme::border());

        let inner = block.inner(log_area);

        let log_lines: Vec<Line> = debug_lines.iter().map(|l| {
            Line::from(Span::styled(format!("  {l}"), theme::dim()))
        }).collect();

        let paragraph = Paragraph::new(log_lines)
            .scroll((state.debug_scroll, 0));

        frame.render_widget(block, log_area);
        frame.render_widget(paragraph, inner);
    }
}
