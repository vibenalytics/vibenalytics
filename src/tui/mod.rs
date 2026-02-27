mod theme;
mod header;
mod overlay;
mod dashboard;
mod sessions;
mod projects;
mod settings;
mod import_picker;
mod onboarding;

use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;

use crate::config::{config_get, config_get_bool, config_get_bool_default, config_set, config_set_bool};
use crate::paths::metrics_path;

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Dashboard,
    Sessions,
    Projects,
    Settings,
}

impl Tab {
    fn index(self) -> usize {
        match self {
            Tab::Dashboard => 0,
            Tab::Sessions => 1,
            Tab::Projects => 2,
            Tab::Settings => 3,
        }
    }

    fn from_index(i: usize) -> Self {
        match i {
            0 => Tab::Dashboard,
            1 => Tab::Sessions,
            2 => Tab::Projects,
            3 => Tab::Settings,
            _ => Tab::Dashboard,
        }
    }

    fn next(self) -> Self {
        Tab::from_index((self.index() + 1).min(3))
    }

    fn prev(self) -> Self {
        Tab::from_index(self.index().saturating_sub(1))
    }
}

struct App {
    dir: PathBuf,
    tab: Tab,
    should_quit: bool,
    user_name: String,
    connected: bool,
    pending_events: usize,
    sessions_state: sessions::SessionsState,
    projects_state: projects::ProjectsState,
    settings_state: settings::SettingsState,
    status_msg: String,
    login_state: Option<crate::auth::LoginListener>,
    import_picker: Option<import_picker::ImportPickerState>,
    onboarding: Option<onboarding::OnboardingState>,
    use_transcripts: bool,
    auto_sync: bool,
    local_sync: bool,
    debug_mode: bool,
    debug_log: Vec<String>,
    last_debug_reload: Instant,
}

impl App {
    fn new(dir: &Path) -> Self {
        let user_name = config_get(dir, "displayName").unwrap_or_else(|| "—".to_string());
        let connected = config_get(dir, "apiKey").is_some();
        let pending_events = std::fs::read_to_string(metrics_path(dir))
            .map(|c| c.lines().filter(|l| !l.trim().is_empty()).count())
            .unwrap_or(0);

        let mut projects_state = projects::ProjectsState::default();
        projects_state.load(dir);

        // Check if onboarding is needed
        let onboarding = if connected {
            let registry = crate::projects::read_projects(dir);
            if !registry.onboarding_completed {
                let ob = onboarding::OnboardingState::new();
                if ob.is_none() {
                    // No projects discovered, mark onboarding done
                    let mut reg = registry;
                    reg.onboarding_completed = true;
                    let _ = crate::projects::write_projects(dir, &reg);
                }
                ob
            } else {
                None
            }
        } else {
            None
        };

        let use_transcripts = config_get(dir, "syncSource")
            .map(|s| s == "transcripts").unwrap_or(false);
        let auto_sync = config_get_bool_default(dir, "autoSync", true);
        let local_sync = config_get_bool(dir, "localSync");
        let debug_mode = config_get_bool(dir, "debugMode");

        App {
            dir: dir.to_path_buf(),
            tab: Tab::Dashboard,
            should_quit: false,
            user_name,
            connected,
            pending_events,
            sessions_state: sessions::SessionsState::default(),
            projects_state,
            settings_state: settings::SettingsState::default(),
            status_msg: String::new(),
            login_state: None,
            import_picker: None,
            onboarding,
            use_transcripts,
            auto_sync,
            local_sync,
            debug_mode,
            debug_log: Vec::new(),
            last_debug_reload: Instant::now(),
        }
    }

    fn reload(&mut self) {
        self.user_name = config_get(&self.dir, "displayName").unwrap_or_else(|| "—".to_string());
        self.connected = config_get(&self.dir, "apiKey").is_some();
        self.pending_events = std::fs::read_to_string(metrics_path(&self.dir))
            .map(|c| c.lines().filter(|l| !l.trim().is_empty()).count())
            .unwrap_or(0);
        self.projects_state.load(&self.dir);
        self.use_transcripts = config_get(&self.dir, "syncSource")
            .map(|s| s == "transcripts").unwrap_or(false);
        self.auto_sync = config_get_bool_default(&self.dir, "autoSync", true);
        self.debug_mode = config_get_bool(&self.dir, "debugMode");
    }

    fn load_debug_log(&mut self) {
        let mut lines = Vec::new();

        // sync.log tail
        let log_path = self.dir.join("sync.log");
        if let Ok(content) = std::fs::read_to_string(&log_path) {
            lines.push("--- sync.log (last 30 lines) ---".to_string());
            let all: Vec<&str> = content.lines().collect();
            let start = all.len().saturating_sub(30);
            for l in &all[start..] {
                lines.push(l.to_string());
            }
        }

        // Latest transcript debug dump
        let debug_dir = self.dir.join("transcript-debug");
        if let Ok(entries) = std::fs::read_dir(&debug_dir) {
            let mut files: Vec<_> = entries
                .flatten()
                .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
                .collect();
            files.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
            if let Some(latest) = files.first() {
                lines.push(String::new());
                lines.push(format!("--- {} ---", latest.file_name().to_string_lossy()));
                if let Ok(content) = std::fs::read_to_string(latest.path()) {
                    for l in content.lines().take(60) {
                        lines.push(l.to_string());
                    }
                    let total = content.lines().count();
                    if total > 60 {
                        lines.push(format!("... ({} more lines)", total - 60));
                    }
                }
            }
        }

        // Cursor state
        let cursors_path = self.dir.join("transcript-cursors.json");
        if let Ok(content) = std::fs::read_to_string(&cursors_path) {
            lines.push(String::new());
            lines.push("--- transcript-cursors.json ---".to_string());
            for l in content.lines().take(30) {
                lines.push(l.to_string());
            }
        }

        self.debug_log = lines;
    }

    fn handle_key(&mut self, key: KeyCode) {
        // Onboarding wizard takes highest priority
        if let Some(ref mut ob) = self.onboarding {
            match &ob.step {
                onboarding::Step::SyncMode => match key {
                    KeyCode::Up => ob.mode_up(),
                    KeyCode::Down => ob.mode_down(),
                    KeyCode::Enter => ob.confirm_mode(),
                    KeyCode::Esc => { self.onboarding = None; }
                    _ => {}
                },
                onboarding::Step::ProjectSelection => match key {
                    KeyCode::Up => ob.up(),
                    KeyCode::Down => ob.down(),
                    KeyCode::Char(' ') => ob.toggle(),
                    KeyCode::Char('a') => ob.select_all(),
                    KeyCode::Char('n') => ob.deselect_all(),
                    KeyCode::Enter => ob.confirm_projects(),
                    KeyCode::Esc => {
                        ob.step = onboarding::Step::SyncMode;
                        ob.cursor = 0;
                        ob.scroll = 0;
                    }
                    _ => {}
                },
                onboarding::Step::ImportPrompt => match key {
                    KeyCode::Up => ob.import_up(),
                    KeyCode::Down => ob.import_down(),
                    KeyCode::Enter => {
                        let do_import = ob.import_cursor == 0;
                        let dir = self.dir.clone();
                        ob.finish(&dir, do_import);
                        // reload immediately for non-import case (skip import / no projects)
                        if !do_import || matches!(&ob.step, onboarding::Step::Done(_)) {
                            self.reload();
                        }
                    }
                    KeyCode::Esc => {
                        ob.step = onboarding::Step::ProjectSelection;
                    }
                    _ => {}
                },
                onboarding::Step::Importing { .. } => {}
                onboarding::Step::Done(_) => {
                    if key == KeyCode::Enter || key == KeyCode::Esc {
                        self.onboarding = None;
                    }
                }
            }
            return;
        }

        // Import picker takes priority
        if let Some(ref mut picker) = self.import_picker {
            match &picker.phase {
                import_picker::ImportPhase::Selecting => {
                    match key {
                        KeyCode::Esc => { self.import_picker = None; }
                        KeyCode::Up => {
                            picker.up();
                        }
                        KeyCode::Down => {
                            picker.down();
                        }
                        KeyCode::Char(' ') => { picker.toggle(); }
                        KeyCode::Char('a') => { picker.select_all(); }
                        KeyCode::Char('n') => { picker.deselect_all(); }
                        KeyCode::Enter => {
                            let dir = self.dir.clone();
                            picker.run_import(&dir);
                        }
                        _ => {}
                    }
                }
                import_picker::ImportPhase::Importing { .. } => {}
                import_picker::ImportPhase::Done(_) => {
                    if key == KeyCode::Esc || key == KeyCode::Enter {
                        self.import_picker = None;
                    }
                }
            }
            return;
        }

        // Login in progress
        if self.login_state.is_some() {
            if key == KeyCode::Esc {
                self.login_state = None;
                self.status_msg = "Login cancelled".into();
            }
            return;
        }

        // Not logged in — only allow login or quit
        if !self.connected {
            match key {
                KeyCode::Enter => self.start_login(),
                KeyCode::Esc => { self.should_quit = true; }
                _ => {}
            }
            return;
        }

        match key {
            KeyCode::Esc => {
                if self.tab == Tab::Projects && self.projects_state.has_changes() {
                    self.projects_state.discard();
                    self.status_msg = "Changes discarded".into();
                } else {
                    self.should_quit = true;
                }
            }
            KeyCode::Left => {
                self.tab = self.tab.prev();
            }
            KeyCode::Right => {
                self.tab = self.tab.next();
            }
            KeyCode::Up => {
                match self.tab {
                    Tab::Sessions => self.sessions_state.up(),
                    Tab::Projects => self.projects_state.up(),
                    Tab::Settings => self.settings_state.up(),
                    _ => {}
                }
            }
            KeyCode::Down => {
                match self.tab {
                    Tab::Sessions => self.sessions_state.down(),
                    Tab::Projects => self.projects_state.down(),
                    Tab::Settings => self.settings_state.down(),
                    _ => {}
                }
            }
            KeyCode::Char(' ') => {
                if self.tab == Tab::Projects {
                    self.projects_state.toggle();
                }
            }
            KeyCode::PageUp => {
                if self.tab == Tab::Settings && self.debug_mode {
                    self.settings_state.scroll_debug_up();
                }
            }
            KeyCode::PageDown => {
                if self.tab == Tab::Settings && self.debug_mode {
                    let visible = 10; // approximate visible lines
                    self.settings_state.scroll_debug_down(self.debug_log.len(), visible);
                }
            }
            KeyCode::Enter => {
                match self.tab {
                    Tab::Settings => {
                        if !self.connected {
                            self.start_login();
                        } else {
                            self.handle_settings_action();
                        }
                    }
                    Tab::Projects => {
                        if self.projects_state.has_changes() {
                            self.status_msg = self.projects_state.save(&self.dir);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn start_login(&mut self) {
        match crate::auth::start_login() {
            Ok(listener) => {
                let port = listener.listener.local_addr().map(|a| a.port()).unwrap_or(0);
                let url = format!("{}/auth/cli?port={port}&state={}", crate::config::DEFAULT_FRONTEND_BASE, listener.nonce);
                self.status_msg = format!("Opening browser... If it didn't open, visit: {url}");
                self.login_state = Some(listener);
            }
            Err(e) => {
                self.status_msg = format!("Login failed: {e}");
            }
        }
    }

    fn handle_settings_action(&mut self) {
        if !self.connected {
            return;
        }
        match self.settings_state.selected {
            0 => {
                let rc = crate::sync::cmd_sync(&self.dir);
                self.status_msg = if rc == 0 {
                    "Sync complete".into()
                } else {
                    "Sync failed — check sync.log".into()
                };
                self.reload();
            }
            1 => {
                match import_picker::ImportPickerState::new() {
                    Some(picker) => {
                        self.import_picker = Some(picker);
                    }
                    None => {
                        self.status_msg = "No projects found in ~/.claude/projects/".into();
                    }
                }
            }
            2 => {
                let new_val = !self.auto_sync;
                if let Ok(()) = config_set_bool(&self.dir, "autoSync", new_val) {
                    self.auto_sync = new_val;
                    let label = if new_val { "enabled" } else { "disabled" };
                    self.status_msg = format!("Auto sync {label}");
                }
            }
            3 => {
                let mut registry = crate::projects::read_projects(&self.dir);
                registry.default_enabled = !registry.default_enabled;
                match crate::projects::write_projects(&self.dir, &registry) {
                    Ok(()) => {
                        let mode = if registry.default_enabled { "auto" } else { "manual" };
                        self.status_msg = format!("Sync mode changed to {mode}");
                        self.reload();
                    }
                    Err(e) => {
                        self.status_msg = format!("Failed to save: {e}");
                    }
                }
            }
            4 => {
                let new_source = if self.use_transcripts { "hooks" } else { "transcripts" };
                if let Ok(()) = config_set(&self.dir, "syncSource", new_source) {
                    self.use_transcripts = !self.use_transcripts;
                    self.status_msg = format!("Data source: {new_source}");
                    self.reload();
                }
            }
            5 => {
                let new_val = !self.local_sync;
                if let Ok(()) = config_set_bool(&self.dir, "localSync", new_val) {
                    self.local_sync = new_val;
                    let label = if new_val { "enabled" } else { "disabled" };
                    self.status_msg = format!("Local sync {label}");
                }
            }
            6 => {
                let new_val = !self.debug_mode;
                if let Ok(()) = config_set_bool(&self.dir, "debugMode", new_val) {
                    self.debug_mode = new_val;
                    if new_val {
                        self.load_debug_log();
                        self.settings_state.debug_scroll = 0;
                        self.status_msg = "Debug mode enabled".into();
                    } else {
                        self.debug_log.clear();
                        self.status_msg = "Debug mode disabled".into();
                    }
                }
            }
            7 => {
                crate::auth::cmd_logout(&self.dir);
                self.status_msg = "Logged out".into();
                self.reload();
            }
            _ => {}
        }
    }

    fn poll_import(&mut self) {
        if let Some(ref mut picker) = self.import_picker {
            let was_importing = matches!(&picker.phase, import_picker::ImportPhase::Importing { .. });
            picker.poll_progress();
            if was_importing && matches!(&picker.phase, import_picker::ImportPhase::Done(_)) {
                self.reload();
            }
        }
        if let Some(ref mut ob) = self.onboarding {
            let was_importing = matches!(&ob.step, onboarding::Step::Importing { .. });
            ob.poll_import();
            if was_importing && matches!(&ob.step, onboarding::Step::Done(_)) {
                self.reload();
            }
        }
    }

    fn poll_login(&mut self) {
        if let Some(ref login) = self.login_state {
            match crate::auth::poll_login(login) {
                Ok(Some((key, name))) => {
                    match crate::auth::save_login(&self.dir, &key, &name) {
                        Ok(()) => {
                            self.status_msg = format!("Logged in as {name}");
                        }
                        Err(e) => {
                            self.status_msg = format!("Login failed: {e}");
                        }
                    }
                    self.login_state = None;
                    self.reload();

                    // Trigger onboarding if not completed
                    let registry = crate::projects::read_projects(&self.dir);
                    if !registry.onboarding_completed {
                        if let Some(ob) = onboarding::OnboardingState::new() {
                            self.onboarding = Some(ob);
                        } else {
                            let mut reg = registry;
                            reg.onboarding_completed = true;
                            let _ = crate::projects::write_projects(&self.dir, &reg);
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    self.status_msg = format!("Login failed: {e}");
                    self.login_state = None;
                }
            }
        }
    }
}

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(stdout))
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) {
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
}

pub fn run(dir: &Path) -> i32 {
    let mut terminal = match setup_terminal() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Failed to initialize terminal: {e}");
            return 1;
        }
    };

    let mut app = App::new(dir);
    let tick_rate = Duration::from_millis(50);
    let mut last_tick = Instant::now();

    loop {
        if app.should_quit {
            break;
        }

        app.poll_login();
        app.poll_import();

        // Reload debug log periodically when viewing settings with debug on
        if app.debug_mode && app.tab == Tab::Settings
            && app.last_debug_reload.elapsed() >= Duration::from_secs(2)
        {
            app.load_debug_log();
            app.last_debug_reload = Instant::now();
        }

        let _ = terminal.draw(|frame| {
            let size = frame.area();

            // Onboarding wizard replaces the whole screen
            if let Some(ref mut ob) = app.onboarding {
                let layout = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(3),  // header (margin + text)
                        Constraint::Min(6),
                        Constraint::Length(2),  // footer (separator + hints)
                    ])
                    .split(size);

                header::render_header(frame, layout[0], &app.user_name, app.connected);
                onboarding::render(frame, layout[1], ob);

                let hints = match &ob.step {
                    onboarding::Step::SyncMode =>
                        "↑/↓ select  enter continue  esc skip",
                    onboarding::Step::ProjectSelection =>
                        "↑/↓ navigate  space toggle  a all  n none  enter confirm  esc back",
                    onboarding::Step::ImportPrompt =>
                        "↑/↓ select  enter confirm  esc back",
                    onboarding::Step::Importing { .. } =>
                        "importing...",
                    onboarding::Step::Done(_) =>
                        "enter continue",
                };
                header::render_footer(frame, layout[2], hints);
                return;
            }

            // Import picker replaces the whole screen
            if let Some(ref mut picker) = app.import_picker {
                let layout = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(3),  // header
                        Constraint::Min(6),    // picker content
                        Constraint::Length(2),  // footer
                    ])
                    .split(size);

                header::render_header(frame, layout[0], &app.user_name, app.connected);
                import_picker::render(frame, layout[1], picker);

                let hints = match &picker.phase {
                    import_picker::ImportPhase::Selecting =>
                        "↑/↓ navigate  space toggle  a all  n none  enter import  esc cancel",
                    import_picker::ImportPhase::Importing { .. } =>
                        "importing...",
                    import_picker::ImportPhase::Done(_) =>
                        "esc return",
                };
                header::render_footer(frame, layout[2], hints);
                return;
            }

            // Not logged in — show login screen
            if !app.connected {
                let layout = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(3),  // header
                        Constraint::Min(6),    // login content
                        Constraint::Length(1),  // status
                        Constraint::Length(2),  // footer
                    ])
                    .split(size);

                header::render_header(frame, layout[0], &app.user_name, app.connected);

                let lines = vec![
                    Line::from(""),
                    Line::from(""),
                    Line::from(Span::styled("  Welcome to Vibenalytics", theme::text())),
                    Line::from(""),
                    Line::from(Span::styled("  Log in to start tracking your Claude Code usage.", theme::dim())),
                    Line::from(""),
                    Line::from(""),
                    Line::from(vec![
                        Span::styled("  > ", theme::accent_bold()),
                        Span::styled("Login", theme::accent_bold()),
                    ]),
                ];
                frame.render_widget(ratatui::widgets::Paragraph::new(lines), layout[1]);

                if !app.status_msg.is_empty() {
                    let status_line = ratatui::widgets::Paragraph::new(
                        Line::from(Span::styled(format!("  {}", &app.status_msg), theme::accent_bold()))
                    );
                    frame.render_widget(status_line, layout[2]);
                }

                let hints = if app.login_state.is_some() {
                    "esc cancel login"
                } else {
                    "enter login  esc quit"
                };
                header::render_footer(frame, layout[3], hints);
                return;
            }

            let has_status = !app.status_msg.is_empty();
            let layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints(if has_status {
                    vec![
                        Constraint::Length(3),  // header (margin + text)
                        Constraint::Length(2),  // tab bar (labels + underline)
                        Constraint::Min(6),     // content
                        Constraint::Length(1),  // status message
                        Constraint::Length(2),  // footer (separator + hints)
                    ]
                } else {
                    vec![
                        Constraint::Length(3),
                        Constraint::Length(2),
                        Constraint::Min(6),
                        Constraint::Length(0),
                        Constraint::Length(2),
                    ]
                })
                .split(size);

            header::render_header(frame, layout[0], &app.user_name, app.connected);
            header::render_tab_bar(frame, layout[1], app.tab.index());

            match app.tab {
                Tab::Dashboard => dashboard::render(frame, layout[2]),
                Tab::Sessions => sessions::render(frame, layout[2], &app.sessions_state),
                Tab::Projects => projects::render(frame, layout[2], &mut app.projects_state),
                Tab::Settings => settings::render(frame, layout[2], &app.settings_state, &app.user_name, app.connected, app.pending_events, app.projects_state.registry.default_enabled, app.use_transcripts, app.auto_sync, app.local_sync, app.debug_mode, &app.debug_log, &app.dir),
            }

            if has_status {
                let status_line = ratatui::widgets::Paragraph::new(
                    Line::from(Span::styled(format!("  {}", &app.status_msg), theme::accent_bold()))
                );
                frame.render_widget(status_line, layout[3]);
            }

            let hints = if app.login_state.is_some() {
                "esc cancel login"
            } else {
                match app.tab {
                    Tab::Dashboard => "←/→ tabs  esc quit",
                    Tab::Sessions => "↑/↓ select  ←/→ tabs  esc quit",
                    Tab::Projects => if app.projects_state.has_changes() {
                        "↑/↓ navigate  space toggle  enter save  esc discard"
                    } else {
                        "↑/↓ navigate  space toggle  ←/→ tabs  esc quit"
                    },
                    Tab::Settings => if app.debug_mode {
                        "↑/↓ select  enter run  PgUp/PgDn scroll log  ←/→ tabs  esc quit"
                    } else {
                        "↑/↓ select  enter run  ←/→ tabs  esc quit"
                    },
                }
            };
            header::render_footer(frame, layout[4], hints);
        });

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout).unwrap_or(false) {
            if let Ok(Event::Key(key)) = event::read() {
                if key.kind == KeyEventKind::Press {
                    app.status_msg.clear();
                    app.handle_key(key.code);
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }
    }

    restore_terminal(&mut terminal);
    0
}
