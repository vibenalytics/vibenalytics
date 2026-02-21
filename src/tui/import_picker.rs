use std::collections::HashSet;
use std::path::Path;
use std::time::SystemTime;
use ratatui::prelude::*;
use super::theme;
use crate::paths::claude_dir;
use crate::transcripts::discover_projects;

pub struct ImportPickerState {
    pub projects: Vec<ImportEntry>,
    pub cursor: usize,
    pub phase: ImportPhase,
    pub scroll: usize,
}

pub struct ImportEntry {
    pub dir_name: String,
    pub display_name: String,
    pub original_path: String,
    pub session_count: usize,
    pub last_active: String,
    pub selected: bool,
}

pub enum ImportPhase {
    Selecting,
    Done(String),
}

fn truncate_path_left(path: &str, max_width: usize) -> String {
    if path.len() <= max_width {
        return path.to_string();
    }
    if max_width <= 2 {
        return "…".to_string();
    }
    let keep = max_width - 1; // 1 for "…"
    format!("…{}", &path[path.len() - keep..])
}

fn format_last_active(ts: Option<SystemTime>) -> String {
    let st = match ts {
        Some(t) => t,
        None => return "—".to_string(),
    };
    let secs = st.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    super::projects::format_elapsed_secs(secs)
}

impl ImportPickerState {
    pub fn new() -> Option<Self> {
        let claude = claude_dir();
        if !claude.exists() {
            return None;
        }

        let discovered = discover_projects(&claude);
        if discovered.is_empty() {
            return None;
        }

        let projects: Vec<ImportEntry> = discovered.into_iter().map(|p| {
            ImportEntry {
                dir_name: p.dir_name,
                display_name: p.display_name,
                original_path: p.original_path,
                session_count: p.session_count,
                last_active: format_last_active(p.last_active),
                selected: true,
            }
        }).collect();

        Some(ImportPickerState {
            projects,
            cursor: 0,
            phase: ImportPhase::Selecting,
            scroll: 0,
        })
    }

    pub fn up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn down(&mut self) {
        let max = self.projects.len().saturating_sub(1);
        if self.cursor < max {
            self.cursor += 1;
        }
    }

    pub fn toggle(&mut self) {
        if self.cursor < self.projects.len() {
            self.projects[self.cursor].selected = !self.projects[self.cursor].selected;
        }
    }

    pub fn select_all(&mut self) {
        for p in &mut self.projects {
            p.selected = true;
        }
    }

    pub fn deselect_all(&mut self) {
        for p in &mut self.projects {
            p.selected = false;
        }
    }

    pub fn selected_count(&self) -> usize {
        self.projects.iter().filter(|p| p.selected).count()
    }

    pub fn selected_session_count(&self) -> usize {
        self.projects.iter().filter(|p| p.selected).map(|p| p.session_count).sum()
    }

    pub fn selected_dir_names(&self) -> HashSet<String> {
        self.projects.iter()
            .filter(|p| p.selected)
            .map(|p| p.dir_name.clone())
            .collect()
    }

    pub fn run_import(&mut self, dir: &Path) {
        let selected = self.selected_dir_names();
        if selected.is_empty() {
            self.phase = ImportPhase::Done("No projects selected".into());
            return;
        }

        match crate::import::run_import(dir, Some(&selected), true) {
            Ok(msg) => self.phase = ImportPhase::Done(msg),
            Err(e) => self.phase = ImportPhase::Done(format!("Error: {e}")),
        }
    }
}

pub fn render(frame: &mut Frame, area: Rect, state: &mut ImportPickerState) {
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled("  Import History — select projects to import", theme::accent_bold())),
        Line::from(""),
    ];

    match &state.phase {
        ImportPhase::Selecting => {
            // Calculate column widths
            let name_width = state.projects.iter()
                .map(|p| p.display_name.len())
                .max()
                .unwrap_or(10)
                .max(7); // min "Project" header width
            let sessions_width = 8;
            let active_width = 11;
            // Fixed columns: "  > ● " (7) + name + "  " + sessions + "  " + active = 7+name+2+8+2+11 = 30+name
            let fixed_cols = 7 + name_width + 2 + sessions_width + 2 + active_width + 4; // +4 for path gap + padding
            let term_w = area.width as usize;
            let path_width = if term_w > fixed_cols + 10 {
                term_w - fixed_cols
            } else {
                0 // hide path column if terminal too narrow
            };

            // Table header
            let mut header_spans = vec![
                Span::styled(format!("       {:<nw$}", "Project", nw = name_width), theme::dim()),
            ];
            if path_width > 0 {
                header_spans.push(Span::styled(format!("  {:<pw$}", "Path", pw = path_width), theme::dim()));
            }
            header_spans.push(Span::styled(format!("  {:>sw$}", "Sessions", sw = sessions_width), theme::dim()));
            header_spans.push(Span::styled(format!("  {:>aw$}", "Last active", aw = active_width), theme::dim()));
            lines.push(Line::from(header_spans));

            // Separator
            let total_w = if path_width > 0 {
                7 + name_width + 2 + path_width + 2 + sessions_width + 2 + active_width
            } else {
                7 + name_width + 2 + sessions_width + 2 + active_width
            };
            lines.push(Line::from(Span::styled(
                format!("  {}", "─".repeat(total_w.min(area.width as usize - 2))),
                theme::dim(),
            )));

            let visible_height = area.height.saturating_sub(9) as usize; // header + table header + separator + footer
            if visible_height > 0 {
                if state.cursor < state.scroll {
                    state.scroll = state.cursor;
                } else if state.cursor >= state.scroll + visible_height {
                    state.scroll = state.cursor - visible_height + 1;
                }
            }
            let end = (state.scroll + visible_height).min(state.projects.len());
            let start = state.scroll.min(state.projects.len());

            for i in start..end {
                let p = &state.projects[i];
                let dot = if p.selected { "●" } else { "○" };
                let dot_style = if p.selected { theme::success() } else { theme::dim() };
                let is_active = i == state.cursor;
                let (marker, name_style) = if is_active {
                    ("> ", theme::accent_bold())
                } else {
                    ("  ", theme::text())
                };
                let dim = if is_active { theme::accent() } else { theme::dim() };

                let mut spans = vec![
                    Span::styled(format!("  {marker}"), name_style),
                    Span::styled(dot, dot_style),
                    Span::styled(format!(" {:<nw$}", p.display_name, nw = name_width), name_style),
                ];
                if path_width > 0 {
                    let truncated = truncate_path_left(&p.original_path, path_width);
                    spans.push(Span::styled(format!("  {:<pw$}", truncated, pw = path_width), dim));
                }
                spans.push(Span::styled(format!("  {:>sw$}", p.session_count, sw = sessions_width), dim));
                spans.push(Span::styled(format!("  {:>aw$}", p.last_active, aw = active_width), dim));
                lines.push(Line::from(spans));
            }

            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!(
                    "  {} projects selected, {} sessions total",
                    state.selected_count(),
                    state.selected_session_count(),
                ),
                theme::dim(),
            )));
        }
        ImportPhase::Done(msg) => {
            lines.push(Line::from(Span::styled(format!("  {msg}"), theme::accent_bold())));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("  Press Esc to return", theme::dim())));
        }
    }

    frame.render_widget(ratatui::widgets::Paragraph::new(lines), area);
}
