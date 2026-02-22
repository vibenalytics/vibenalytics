use std::collections::HashSet;
use std::path::Path;
use std::sync::mpsc;
use std::time::SystemTime;
use ratatui::prelude::*;
use super::theme;
use crate::paths::claude_dir;
use crate::import::ImportProgress;
use crate::transcripts::{discover_projects, discover_sessions, parse_session_transcript};

// ---- Steps ----

pub enum Step {
    SyncMode,
    ProjectSelection,
    ImportPrompt,
    Importing {
        rx: mpsc::Receiver<ImportProgress>,
        status: String,
    },
    Done(String),
}

#[derive(Clone, Copy, PartialEq)]
pub enum SyncMode {
    Auto,
    Manual,
}

// ---- Entry ----

pub struct ProjectEntry {
    pub dir_name: String,
    pub display_name: String,
    pub original_path: String,
    pub path_hash: String,
    pub session_count: usize,
    pub last_active: String,
    pub selected: bool,
}

// ---- Import summary stats ----

struct ImportStats {
    projects: usize,
    sessions: usize,
    prompts: u32,
    tool_calls: u32,
}

// ---- State ----

pub struct OnboardingState {
    pub step: Step,
    pub sync_mode: SyncMode,
    pub mode_cursor: usize,
    pub projects: Vec<ProjectEntry>,
    pub cursor: usize,
    pub scroll: usize,
    pub import_cursor: usize, // 0 = Yes, 1 = No
    import_stats: Option<ImportStats>,
}

// ---- Helpers ----

fn truncate_path_left(path: &str, max_width: usize) -> String {
    if path.len() <= max_width {
        return path.to_string();
    }
    if max_width <= 2 {
        return "…".to_string();
    }
    let keep = max_width - 1;
    format!("…{}", &path[path.len() - keep..])
}

fn format_last_active(ts: Option<SystemTime>) -> String {
    let st = match ts {
        Some(t) => t,
        None => return "—".to_string(),
    };
    let elapsed = match st.elapsed() {
        Ok(d) => d,
        Err(_) => return "just now".to_string(),
    };
    let secs = elapsed.as_secs();
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else if secs < 86400 * 30 {
        format!("{}d ago", secs / 86400)
    } else if secs < 86400 * 365 {
        format!("{}mo ago", secs / (86400 * 30))
    } else {
        format!("{}y ago", secs / (86400 * 365))
    }
}

fn compute_import_stats(selected_dirs: &HashSet<String>) -> ImportStats {
    let claude = claude_dir();
    let sessions_list = discover_sessions(&claude, Some(selected_dirs));

    let mut total_sessions = 0usize;
    let mut total_prompts = 0u32;
    let mut total_tools = 0u32;

    for (project_name, ph, path) in &sessions_list {
        if let Some(session) = parse_session_transcript(path, project_name, ph) {
            total_sessions += 1;
            total_prompts += session.prompt_count;
            total_tools += session.tools.values().sum::<u32>();
        }
    }

    ImportStats {
        projects: selected_dirs.len(),
        sessions: total_sessions,
        prompts: total_prompts,
        tool_calls: total_tools,
    }
}

// ---- Implementation ----

impl OnboardingState {
    pub fn new() -> Option<Self> {
        let claude = claude_dir();
        if !claude.exists() {
            return None;
        }

        let discovered = discover_projects(&claude);
        if discovered.is_empty() {
            return None;
        }

        let projects: Vec<ProjectEntry> = discovered.into_iter().map(|p| {
            ProjectEntry {
                dir_name: p.dir_name,
                display_name: p.display_name,
                original_path: p.original_path,
                path_hash: p.path_hash,
                session_count: p.session_count,
                last_active: format_last_active(p.last_active),
                selected: true,
            }
        }).collect();

        Some(OnboardingState {
            step: Step::SyncMode,
            sync_mode: SyncMode::Auto,
            mode_cursor: 0,
            projects,
            cursor: 0,
            scroll: 0,
            import_cursor: 0,
            import_stats: None,
        })
    }

    // -- SyncMode navigation --

    pub fn mode_up(&mut self) { self.mode_cursor = 0; }
    pub fn mode_down(&mut self) { self.mode_cursor = 1; }

    pub fn confirm_mode(&mut self) {
        self.sync_mode = if self.mode_cursor == 0 { SyncMode::Auto } else { SyncMode::Manual };
        let select = self.sync_mode == SyncMode::Auto;
        for p in &mut self.projects {
            p.selected = select;
        }
        self.cursor = 0;
        self.scroll = 0;
        self.step = Step::ProjectSelection;
    }

    // -- ProjectSelection navigation --

    pub fn up(&mut self) { self.cursor = self.cursor.saturating_sub(1); }

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
        for p in &mut self.projects { p.selected = true; }
    }

    pub fn deselect_all(&mut self) {
        for p in &mut self.projects { p.selected = false; }
    }

    pub fn selected_count(&self) -> usize {
        self.projects.iter().filter(|p| p.selected).count()
    }

    pub fn selected_session_count(&self) -> usize {
        self.projects.iter().filter(|p| p.selected).map(|p| p.session_count).sum()
    }

    fn selected_dir_names(&self) -> HashSet<String> {
        self.projects.iter()
            .filter(|p| p.selected)
            .map(|p| p.dir_name.clone())
            .collect()
    }

    // -- Confirm projects, compute stats, show import prompt --

    pub fn confirm_projects(&mut self) {
        let selected = self.selected_dir_names();
        if selected.is_empty() {
            self.import_stats = Some(ImportStats {
                projects: 0, sessions: 0, prompts: 0, tool_calls: 0,
            });
        } else {
            self.import_stats = Some(compute_import_stats(&selected));
        }
        self.import_cursor = 0;
        self.step = Step::ImportPrompt;
    }

    // -- ImportPrompt navigation --

    pub fn import_up(&mut self) { self.import_cursor = 0; }
    pub fn import_down(&mut self) { self.import_cursor = 1; }

    // -- Finalize --

    pub fn finish(&mut self, dir: &Path, do_import: bool) {
        let default_enabled = self.sync_mode == SyncMode::Auto;

        // Build selections for bulk registration
        let selections: Vec<(String, String, String, bool)> = self.projects.iter().map(|p| {
            (p.display_name.clone(), p.original_path.clone(), p.path_hash.clone(), p.selected)
        }).collect();

        if let Err(e) = crate::projects::register_projects_bulk(dir, &selections, default_enabled) {
            self.step = Step::Done(format!("Error saving projects: {e}"));
            return;
        }

        if !do_import {
            self.step = Step::Done("Setup complete!".to_string());
            return;
        }

        let selected_dirs = self.selected_dir_names();
        if selected_dirs.is_empty() {
            self.step = Step::Done("Setup complete! No projects to import.".to_string());
            return;
        }

        let rx = crate::import::start_import(dir.to_path_buf(), selected_dirs);
        self.step = Step::Importing {
            rx,
            status: "Starting import...".into(),
        };
    }

    pub fn poll_import(&mut self) {
        let next_step = if let Step::Importing { rx, status } = &mut self.step {
            loop {
                match rx.try_recv() {
                    Ok(ImportProgress::Parsing { total_files }) => {
                        *status = format!("Parsing {} session files...", total_files);
                    }
                    Ok(ImportProgress::Syncing { batch, total_batches }) => {
                        *status = format!("Syncing batch {}/{}...", batch, total_batches);
                    }
                    Ok(ImportProgress::Done(msg)) => {
                        break Some(Step::Done(format!("Setup complete! {msg}")));
                    }
                    Err(_) => break None,
                }
            }
        } else {
            None
        };
        if let Some(step) = next_step {
            self.step = step;
        }
    }
}

// ---- Rendering ----

pub fn render(frame: &mut Frame, area: Rect, state: &mut OnboardingState) {
    match &state.step {
        Step::SyncMode => render_sync_mode(frame, area, state),
        Step::ProjectSelection => render_project_selection(frame, area, state),
        Step::ImportPrompt => render_import_prompt(frame, area, state),
        Step::Importing { status, .. } => render_importing(frame, area, status),
        Step::Done(msg) => render_done(frame, area, msg),
    }
}

fn render_sync_mode(frame: &mut Frame, area: Rect, state: &OnboardingState) {
    let options = [
        ("Auto-sync all projects", "New projects are automatically synced"),
        ("Manual — I'll choose which projects to sync", "New projects are paused by default"),
    ];

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled("  Welcome to Vibenalytics!", theme::accent_bold())),
        Line::from(""),
        Line::from(Span::styled("  How should new projects be handled?", theme::text())),
        Line::from(""),
    ];

    for (i, (label, desc)) in options.iter().enumerate() {
        let is_active = i == state.mode_cursor;
        let dot = if is_active { "●" } else { "○" };
        let dot_style = if is_active { theme::success() } else { theme::dim() };
        let (marker, style) = if is_active {
            ("> ", theme::accent_bold())
        } else {
            ("  ", theme::text())
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {marker}"), style),
            Span::styled(dot, dot_style),
            Span::styled(format!(" {label}"), style),
        ]));
        lines.push(Line::from(Span::styled(format!("       {desc}"), theme::dim())));
        lines.push(Line::from(""));
    }

    frame.render_widget(ratatui::widgets::Paragraph::new(lines), area);
}

fn render_project_selection(frame: &mut Frame, area: Rect, state: &mut OnboardingState) {
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled("  Select projects to sync", theme::accent_bold())),
        Line::from(""),
    ];

    let name_width = state.projects.iter()
        .map(|p| p.display_name.len())
        .max()
        .unwrap_or(10)
        .max(7);
    let sessions_width = 8;
    let active_width = 11;
    let fixed_cols = 7 + name_width + 2 + sessions_width + 2 + active_width + 4;
    let term_w = area.width as usize;
    let path_width = if term_w > fixed_cols + 10 { term_w - fixed_cols } else { 0 };

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

    let total_w = if path_width > 0 {
        7 + name_width + 2 + path_width + 2 + sessions_width + 2 + active_width
    } else {
        7 + name_width + 2 + sessions_width + 2 + active_width
    };
    lines.push(Line::from(Span::styled(
        format!("  {}", "─".repeat(total_w.min(area.width as usize - 2))),
        theme::dim(),
    )));

    let visible_height = area.height.saturating_sub(9) as usize;
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
        format!("  {} projects selected, {} sessions total",
            state.selected_count(),
            state.selected_session_count(),
        ),
        theme::dim(),
    )));

    frame.render_widget(ratatui::widgets::Paragraph::new(lines), area);
}

fn render_import_prompt(frame: &mut Frame, area: Rect, state: &OnboardingState) {
    let stats = state.import_stats.as_ref();
    let sessions = stats.map(|s| s.sessions).unwrap_or(0);
    let prompts = stats.map(|s| s.prompts).unwrap_or(0);
    let tool_calls = stats.map(|s| s.tool_calls).unwrap_or(0);
    let proj_count = stats.map(|s| s.projects).unwrap_or(0);

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled("  Import from agent history?", theme::accent_bold())),
        Line::from(""),
        Line::from(Span::styled(
            format!("  Found data for {proj_count} selected projects:"),
            theme::text(),
        )),
        Line::from(""),
        Line::from(Span::styled(format!("    {sessions} sessions"), theme::text())),
        Line::from(Span::styled(format!("    {prompts} prompts"), theme::text())),
        Line::from(Span::styled(format!("    {tool_calls} tool calls"), theme::text())),
        Line::from(""),
    ];

    let options = ["Yes, import and sync", "No, skip import"];
    for (i, label) in options.iter().enumerate() {
        let is_active = i == state.import_cursor;
        let dot = if is_active { "●" } else { "○" };
        let dot_style = if is_active { theme::success() } else { theme::dim() };
        let (marker, style) = if is_active {
            ("> ", theme::accent_bold())
        } else {
            ("  ", theme::text())
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {marker}"), style),
            Span::styled(dot, dot_style),
            Span::styled(format!(" {label}"), style),
        ]));
    }

    frame.render_widget(ratatui::widgets::Paragraph::new(lines), area);
}

fn render_importing(frame: &mut Frame, area: Rect, status: &str) {
    let spinner = theme::spinner_char();
    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(format!("  {spinner} "), theme::accent_bold()),
            Span::styled("Importing history...", theme::accent_bold()),
        ]),
        Line::from(""),
        Line::from(Span::styled(format!("    {status}"), theme::dim())),
    ];
    frame.render_widget(ratatui::widgets::Paragraph::new(lines), area);
}

fn render_done(frame: &mut Frame, area: Rect, msg: &str) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(format!("  {msg}"), theme::accent_bold())),
        Line::from(""),
        Line::from(Span::styled("  Press Enter to continue", theme::dim())),
    ];
    frame.render_widget(ratatui::widgets::Paragraph::new(lines), area);
}
