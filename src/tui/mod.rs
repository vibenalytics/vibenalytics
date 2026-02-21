mod theme;
mod header;
mod overlay;
mod dashboard;
mod sessions;
mod projects;
mod settings;
mod import_picker;

use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;

use crate::config::config_get;
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
        }
    }

    fn reload(&mut self) {
        self.user_name = config_get(&self.dir, "displayName").unwrap_or_else(|| "—".to_string());
        self.connected = config_get(&self.dir, "apiKey").is_some();
        self.pending_events = std::fs::read_to_string(metrics_path(&self.dir))
            .map(|c| c.lines().filter(|l| !l.trim().is_empty()).count())
            .unwrap_or(0);
        self.projects_state.load(&self.dir);
    }

    fn handle_key(&mut self, key: KeyCode) {
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
                            self.reload();
                        }
                        _ => {}
                    }
                }
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

        match key {
            KeyCode::Esc => {
                self.should_quit = true;
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
            KeyCode::Enter => {
                match self.tab {
                    Tab::Settings => self.handle_settings_action(),
                    Tab::Projects => self.handle_project_toggle(),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn handle_settings_action(&mut self) {
        match self.settings_state.selected {
            0 => {
                match crate::auth::start_login() {
                    Ok(listener) => {
                        self.login_state = Some(listener);
                        self.status_msg = "Waiting for browser authorization... (Esc to cancel)".into();
                    }
                    Err(e) => {
                        self.status_msg = format!("Login failed: {e}");
                    }
                }
            }
            1 => {
                let rc = crate::sync::cmd_sync(&self.dir);
                self.status_msg = if rc == 0 {
                    "Sync complete".into()
                } else {
                    "Sync failed — check sync.log".into()
                };
                self.reload();
            }
            2 => {
                match import_picker::ImportPickerState::new() {
                    Some(picker) => {
                        self.import_picker = Some(picker);
                    }
                    None => {
                        self.status_msg = "No projects found in ~/.claude/projects/".into();
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_project_toggle(&mut self) {
        let idx = self.projects_state.selected;
        if idx >= self.projects_state.registry.projects.len() {
            return;
        }
        let p = &self.projects_state.registry.projects[idx];
        let name = p.name.clone();
        if p.enabled {
            match crate::projects::disable_project(&self.dir, &name) {
                Ok(n) => self.status_msg = format!("Paused \"{}\"", n),
                Err(e) => self.status_msg = e,
            }
        } else {
            match crate::projects::enable_project(&self.dir, &name) {
                Ok(n) => self.status_msg = format!("Resumed \"{}\"", n),
                Err(e) => self.status_msg = e,
            }
        }
        self.projects_state.load(&self.dir);
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

        let _ = terminal.draw(|frame| {
            let size = frame.area();

            // Import picker replaces the whole screen
            if let Some(ref mut picker) = app.import_picker {
                let layout = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),  // header
                        Constraint::Min(6),    // picker content
                        Constraint::Length(1),  // footer
                    ])
                    .split(size);

                header::render_header(frame, layout[0], &app.user_name, app.connected);
                import_picker::render(frame, layout[1], picker);

                let hints = match &picker.phase {
                    import_picker::ImportPhase::Selecting =>
                        "↑/↓ navigate  space toggle  a all  n none  enter import  esc cancel",
                    import_picker::ImportPhase::Done(_) =>
                        "esc return",
                };
                header::render_footer(frame, layout[2], hints);
                return;
            }

            let has_status = !app.status_msg.is_empty();
            let layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints(if has_status {
                    vec![
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Min(6),
                        Constraint::Length(1),
                        Constraint::Length(1),
                    ]
                } else {
                    vec![
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Min(6),
                        Constraint::Length(0),
                        Constraint::Length(1),
                    ]
                })
                .split(size);

            header::render_header(frame, layout[0], &app.user_name, app.connected);
            header::render_tab_bar(frame, layout[1], app.tab.index());

            match app.tab {
                Tab::Dashboard => dashboard::render(frame, layout[2]),
                Tab::Sessions => sessions::render(frame, layout[2], &app.sessions_state),
                Tab::Projects => projects::render(frame, layout[2], &app.projects_state),
                Tab::Settings => settings::render(frame, layout[2], &app.settings_state, &app.user_name, app.connected, app.pending_events),
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
                    Tab::Projects => "↑/↓ select  enter toggle  ←/→ tabs  esc quit",
                    Tab::Settings => "↑/↓ select  enter run  ←/→ tabs  esc quit",
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
