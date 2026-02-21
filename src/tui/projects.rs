use ratatui::prelude::*;
use super::theme;
use crate::projects::{ProjectRegistry, read_projects};
use std::path::Path;

pub struct ProjectsState {
    pub selected: usize,
    pub registry: ProjectRegistry,
}

impl Default for ProjectsState {
    fn default() -> Self {
        ProjectsState {
            selected: 0,
            registry: ProjectRegistry::default(),
        }
    }
}

impl ProjectsState {
    pub fn load(&mut self, dir: &Path) {
        self.registry = read_projects(dir);
    }

    pub fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn down(&mut self) {
        let max = self.registry.projects.len().saturating_sub(1);
        if self.selected < max {
            self.selected += 1;
        }
    }
}

pub fn render(frame: &mut Frame, area: Rect, state: &ProjectsState) {
    let mut lines = vec![Line::from("")];

    if state.registry.projects.is_empty() {
        lines.push(Line::from(Span::styled("  No projects tracked.", theme::dim())));
        lines.push(Line::from(Span::styled("  Run: vibenalytics project add", theme::dim())));
    } else {
        for (i, p) in state.registry.projects.iter().enumerate() {
            let dot = if p.enabled { "●" } else { "○" };
            let dot_style = if p.enabled { theme::success() } else { theme::dim() };
            let (marker, name_style) = if i == state.selected {
                ("> ", theme::accent_bold())
            } else {
                ("  ", theme::text())
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {marker}"), name_style),
                Span::styled(dot, dot_style),
                Span::styled(format!(" {}", p.name), name_style),
                Span::styled(format!("  {}", p.path), theme::dim()),
            ]));
        }
    }

    frame.render_widget(ratatui::widgets::Paragraph::new(lines), area);
}
