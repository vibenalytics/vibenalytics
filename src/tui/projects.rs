use std::collections::HashMap;
use std::path::Path;
use std::time::SystemTime;
use ratatui::prelude::*;
use super::theme;
use crate::projects::{ProjectRegistry, read_projects};
use crate::paths::claude_dir;
use crate::transcripts::discover_projects;

pub struct ProjectsState {
    pub selected: usize,
    pub scroll: usize,
    pub registry: ProjectRegistry,
    pub enriched: Vec<EnrichedProject>,
}

pub struct EnrichedProject {
    pub name: String,
    pub path: String,
    pub enabled: bool,
    pub session_count: usize,
    pub last_active: String,
    pub sort_ts: i64,
    pub registry_idx: usize, // index into registry.projects for save/discard
}

impl Default for ProjectsState {
    fn default() -> Self {
        ProjectsState {
            selected: 0,
            scroll: 0,
            registry: ProjectRegistry::default(),
            enriched: Vec::new(),
        }
    }
}

/// Resolve the best available timestamp for a project and return (display_string, unix_ts).
/// Prefers transcript file modification time, falls back to added_at from registry.
fn resolve_activity(last_active: Option<SystemTime>, added_at: &str) -> (String, i64) {
    // Try transcript-based timestamp first
    if let Some(st) = last_active {
        let unix = st.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        return (format_elapsed_secs(st.elapsed().map(|d| d.as_secs()).unwrap_or(0)), unix);
    }
    // Fall back to added_at from project registry
    if !added_at.is_empty() {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(added_at, "%Y-%m-%dT%H:%M:%SZ") {
            let then = naive.and_utc();
            let unix = then.timestamp();
            let secs = (chrono::Utc::now() - then).num_seconds().max(0) as u64;
            return (format_elapsed_secs(secs), unix);
        }
    }
    ("—".to_string(), 0)
}

pub fn format_elapsed_secs(secs: u64) -> String {
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

impl ProjectsState {
    pub fn load(&mut self, dir: &Path) {
        self.registry = read_projects(dir);

        // Build hash -> discovered data lookup
        let claude = claude_dir();
        let discovered = if claude.exists() { discover_projects(&claude) } else { Vec::new() };
        let mut discovered_map: HashMap<String, (usize, Option<SystemTime>)> = HashMap::new();
        for d in &discovered {
            discovered_map.insert(d.path_hash.clone(), (d.session_count, d.last_active));
        }

        self.enriched = self.registry.projects.iter().enumerate().map(|(idx, p)| {
            let (session_count, last_active_ts) = discovered_map
                .get(&p.path_hash)
                .cloned()
                .unwrap_or((0, None));
            let (last_active, sort_ts) = resolve_activity(last_active_ts, &p.added_at);
            EnrichedProject {
                name: p.name.clone(),
                path: p.path.clone(),
                enabled: p.enabled,
                session_count,
                last_active,
                sort_ts,
                registry_idx: idx,
            }
        }).collect();

        // Sort by most recent first
        self.enriched.sort_by(|a, b| b.sort_ts.cmp(&a.sort_ts));
    }

    pub fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn down(&mut self) {
        let max = self.enriched.len().saturating_sub(1);
        if self.selected < max {
            self.selected += 1;
        }
    }

    pub fn toggle(&mut self) {
        if self.selected < self.enriched.len() {
            self.enriched[self.selected].enabled = !self.enriched[self.selected].enabled;
        }
    }

    pub fn has_changes(&self) -> bool {
        self.enriched.iter().any(|e| e.enabled != self.registry.projects[e.registry_idx].enabled)
    }

    pub fn save(&mut self, dir: &Path) -> String {
        let mut registry = self.registry.clone();
        let mut changed = 0usize;
        for e in &self.enriched {
            if e.enabled != registry.projects[e.registry_idx].enabled {
                registry.projects[e.registry_idx].enabled = e.enabled;
                changed += 1;
            }
        }
        match crate::projects::write_projects(dir, &registry) {
            Ok(()) => {
                self.registry = registry;
                format!("Saved {changed} change{}", if changed == 1 { "" } else { "s" })
            }
            Err(e) => format!("Save failed: {e}"),
        }
    }

    pub fn discard(&mut self) {
        for e in self.enriched.iter_mut() {
            e.enabled = self.registry.projects[e.registry_idx].enabled;
        }
    }
}

pub fn render(frame: &mut Frame, area: Rect, state: &mut ProjectsState) {
    let mut lines = vec![Line::from("")];

    if state.enriched.is_empty() {
        lines.push(Line::from(Span::styled("  No projects tracked.", theme::dim())));
        lines.push(Line::from(Span::styled("  Run: vibenalytics project add", theme::dim())));
        frame.render_widget(ratatui::widgets::Paragraph::new(lines), area);
        return;
    }

    // Column widths
    let name_width = state.enriched.iter()
        .map(|p| p.name.len())
        .max()
        .unwrap_or(7)
        .max(7);
    let sessions_width = 8;
    let active_width = 11;
    let status_width = 6; // "Status"
    let fixed_cols = 7 + name_width + 2 + status_width + 2 + sessions_width + 2 + active_width + 4;
    let term_w = area.width as usize;
    let path_width = if term_w > fixed_cols + 10 { term_w - fixed_cols } else { 0 };

    // Table header
    let mut header_spans = vec![
        Span::styled(format!("       {:<nw$}", "Project", nw = name_width), theme::dim()),
    ];
    if path_width > 0 {
        header_spans.push(Span::styled(format!("  {:<pw$}", "Path", pw = path_width), theme::dim()));
    }
    header_spans.push(Span::styled(format!("  {:>stw$}", "Status", stw = status_width), theme::dim()));
    header_spans.push(Span::styled(format!("  {:>sw$}", "Sessions", sw = sessions_width), theme::dim()));
    header_spans.push(Span::styled(format!("  {:>aw$}", "Last active", aw = active_width), theme::dim()));
    lines.push(Line::from(header_spans));

    // Separator
    let total_w = if path_width > 0 {
        7 + name_width + 2 + path_width + 2 + status_width + 2 + sessions_width + 2 + active_width
    } else {
        7 + name_width + 2 + status_width + 2 + sessions_width + 2 + active_width
    };
    lines.push(Line::from(Span::styled(
        format!("  {}", "─".repeat(total_w.min(area.width as usize - 2))),
        theme::dim(),
    )));

    // Scrolling
    let visible_height = area.height.saturating_sub(6) as usize; // blank + header + separator + blank + summary
    if visible_height > 0 {
        if state.selected < state.scroll {
            state.scroll = state.selected;
        } else if state.selected >= state.scroll + visible_height {
            state.scroll = state.selected - visible_height + 1;
        }
    }
    let end = (state.scroll + visible_height).min(state.enriched.len());
    let start = state.scroll.min(state.enriched.len());

    for i in start..end {
        let p = &state.enriched[i];
        let dot = if p.enabled { "●" } else { "○" };
        let dot_style = if p.enabled { theme::success() } else { theme::dim() };
        let is_active = i == state.selected;
        let (marker, name_style) = if is_active {
            ("> ", theme::accent_bold())
        } else {
            ("  ", theme::text())
        };
        let dim = if is_active { theme::accent() } else { theme::dim() };

        let status_text = if p.enabled { "active" } else { "paused" };
        let status_style = if p.enabled {
            if is_active { theme::accent() } else { theme::success() }
        } else {
            theme::dim()
        };

        let mut spans = vec![
            Span::styled(format!("  {marker}"), name_style),
            Span::styled(dot, dot_style),
            Span::styled(format!(" {:<nw$}", p.name, nw = name_width), name_style),
        ];
        if path_width > 0 {
            let truncated = truncate_path_left(&p.path, path_width);
            spans.push(Span::styled(format!("  {:<pw$}", truncated, pw = path_width), dim));
        }
        spans.push(Span::styled(format!("  {:>stw$}", status_text, stw = status_width), status_style));
        spans.push(Span::styled(format!("  {:>sw$}", p.session_count, sw = sessions_width), dim));
        spans.push(Span::styled(format!("  {:>aw$}", p.last_active, aw = active_width), dim));
        lines.push(Line::from(spans));
    }

    let mode_label = if state.registry.default_enabled { "auto" } else { "manual" };
    lines.push(Line::from(""));
    if state.has_changes() {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {} projects, sync mode: {mode_label}", state.enriched.len()),
                theme::dim(),
            ),
            Span::styled("  (unsaved changes)", theme::warning()),
        ]));
    } else {
        lines.push(Line::from(Span::styled(
            format!("  {} projects, sync mode: {mode_label}", state.enriched.len()),
            theme::dim(),
        )));
    }

    frame.render_widget(ratatui::widgets::Paragraph::new(lines), area);
}
