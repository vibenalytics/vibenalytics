use ratatui::prelude::*;
use super::theme;

pub struct SessionsState {
    pub selected: usize,
}

impl Default for SessionsState {
    fn default() -> Self {
        SessionsState { selected: 0 }
    }
}

impl SessionsState {
    pub fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }
    pub fn down(&mut self) {
        self.selected += 1;
    }
}

pub fn render(frame: &mut Frame, area: Rect, _state: &SessionsState) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled("  No sessions loaded yet.", theme::dim())),
        Line::from(Span::styled("  Run 'vibenalytics sync' to populate.", theme::dim())),
    ];

    frame.render_widget(ratatui::widgets::Paragraph::new(lines), area);
}
