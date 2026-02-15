use std::io::{self, Stdout};
use std::path::Path;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    widgets::*,
};

// ---- Menu data ----

const MAIN_MENU_ITEMS: &[&str] = &[
    " Dashboard",
    " Sessions",
    " Projects",
    " Tools",
    " Settings",
    " Quit",
];

const MAIN_MENU_DESCRIPTIONS: &[&str] = &[
    "View usage overview, daily trends, and weekly summaries",
    "Browse active and historical Claude Code sessions",
    "Explore project-level analytics and activity heatmaps",
    "Analyze tool usage, latency, and permission patterns",
    "Configure sync, API keys, and display preferences",
    "Exit the dashboard",
];

const SUB_MENUS: &[&[&str]] = &[
    &["Overview", "Daily Trends", "Weekly Summary", "Export Data"],
    &["Active Sessions", "Session History", "Session Details"],
    &["All Projects", "Project Comparison", "Activity Heatmap"],
    &["Tool Usage Stats", "Latency Analysis", "Permission Requests"],
    &["Sync Config", "API Key Management", "Preferences"],
];

const SUB_MENU_ICONS: &[&[&str]] = &[
    &["  ", "  ", "  ", "  "],
    &["  ", "  ", "  "],
    &["  ", "  ", "  "],
    &["  ", "  ", "  "],
    &["  ", "  ", "  "],
];

// ---- App state ----

#[derive(Clone, PartialEq)]
enum Screen {
    Welcome,
    MainMenu,
    SubMenu(usize),
    Detail(usize, usize),
}

struct App {
    screen: Screen,
    menu_index: usize,
    sub_index: usize,
    should_quit: bool,
    welcome_frame: usize,
    config_status: String,
    metrics_lines: usize,
}

impl App {
    fn new(dir: &Path) -> Self {
        let config_status = if dir.join(".sync-config.json").exists() {
            "Connected".to_string()
        } else {
            "Not configured".to_string()
        };

        let metrics_lines = std::fs::read_to_string(dir.join("metrics.jsonl"))
            .map(|c| c.lines().filter(|l| !l.trim().is_empty()).count())
            .unwrap_or(0);

        App {
            screen: Screen::Welcome,
            menu_index: 0,
            sub_index: 0,
            should_quit: false,
            welcome_frame: 0,
            config_status,
            metrics_lines,
        }
    }

    fn menu_len(&self) -> usize {
        match &self.screen {
            Screen::Welcome => 0,
            Screen::MainMenu => MAIN_MENU_ITEMS.len(),
            Screen::SubMenu(parent) => SUB_MENUS.get(*parent).map_or(0, |m| m.len() + 1), // +1 for Back
            Screen::Detail(..) => 0,
        }
    }

    fn handle_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Char('q') | KeyCode::Esc => {
                match &self.screen {
                    Screen::Welcome => self.should_quit = true,
                    Screen::MainMenu => self.should_quit = true,
                    Screen::SubMenu(_) => {
                        self.screen = Screen::MainMenu;
                        self.sub_index = 0;
                    }
                    Screen::Detail(parent, _) => {
                        let p = *parent;
                        self.screen = Screen::SubMenu(p);
                    }
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                match &self.screen {
                    Screen::MainMenu => {
                        if self.menu_index > 0 {
                            self.menu_index -= 1;
                        } else {
                            self.menu_index = MAIN_MENU_ITEMS.len() - 1;
                        }
                    }
                    Screen::SubMenu(_) => {
                        let len = self.menu_len();
                        if self.sub_index > 0 {
                            self.sub_index -= 1;
                        } else {
                            self.sub_index = len - 1;
                        }
                    }
                    _ => {}
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                match &self.screen {
                    Screen::MainMenu => {
                        if self.menu_index < MAIN_MENU_ITEMS.len() - 1 {
                            self.menu_index += 1;
                        } else {
                            self.menu_index = 0;
                        }
                    }
                    Screen::SubMenu(_) => {
                        let len = self.menu_len();
                        if self.sub_index < len - 1 {
                            self.sub_index += 1;
                        } else {
                            self.sub_index = 0;
                        }
                    }
                    _ => {}
                }
            }
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                match &self.screen {
                    Screen::Welcome => {
                        self.screen = Screen::MainMenu;
                    }
                    Screen::MainMenu => {
                        if self.menu_index == MAIN_MENU_ITEMS.len() - 1 {
                            self.should_quit = true;
                        } else if self.menu_index < SUB_MENUS.len() {
                            self.screen = Screen::SubMenu(self.menu_index);
                            self.sub_index = 0;
                        }
                    }
                    Screen::SubMenu(parent) => {
                        let p = *parent;
                        let sub_len = SUB_MENUS.get(p).map_or(0, |m| m.len());
                        if self.sub_index == sub_len {
                            // "Back" item
                            self.screen = Screen::MainMenu;
                            self.sub_index = 0;
                        } else {
                            self.screen = Screen::Detail(p, self.sub_index);
                        }
                    }
                    Screen::Detail(parent, _) => {
                        let p = *parent;
                        self.screen = Screen::SubMenu(p);
                    }
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                match &self.screen {
                    Screen::SubMenu(_) => {
                        self.screen = Screen::MainMenu;
                        self.sub_index = 0;
                    }
                    Screen::Detail(parent, _) => {
                        let p = *parent;
                        self.screen = Screen::SubMenu(p);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

// ---- Rendering ----

const LOGO: &[&str] = &[
    "  _____ _                 _             _       _   _          ",
    " / ____| |               | |           | |     | | (_)         ",
    "| |    | | __ _ _   _  __| |_ __   __ _| |_   _| |_ _  ___ ___ ",
    "| |    | |/ _` | | | |/ _` | '_ \\ / _` | | | | | __| |/ __/ __|",
    "| |____| | (_| | |_| | (_| | | | | (_| | | |_| | |_| | (__\\__ \\",
    " \\_____|_|\\__,_|\\__,_|\\__,_|_| |_|\\__,_|_|\\__, |\\__|_|\\___|___/",
    "                                            __/ |              ",
    "                                           |___/               ",
];

fn render_header(frame: &mut Frame, area: Rect) {
    let title = Line::from(vec![
        Span::styled("  claudnalytics", Style::default().fg(Color::Cyan).bold()),
        Span::styled("  v2.0", Style::default().fg(Color::DarkGray)),
    ]);
    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(title)
        .title_alignment(Alignment::Left);
    frame.render_widget(block, area);
}

fn render_footer(frame: &mut Frame, area: Rect, screen: &Screen) {
    let keys = match screen {
        Screen::Welcome => "Enter: Start  |  q: Quit",
        Screen::MainMenu => "j/k: Navigate  |  Enter: Select  |  q: Quit",
        Screen::SubMenu(_) => "j/k: Navigate  |  Enter: Select  |  Esc: Back  |  q: Quit",
        Screen::Detail(..) => "Enter/Esc: Back  |  q: Quit",
    };

    let footer = Paragraph::new(Line::from(vec![
        Span::styled("  ", Style::default().fg(Color::DarkGray)),
        Span::styled(keys, Style::default().fg(Color::DarkGray)),
    ]))
    .block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(footer, area);
}

fn render_welcome(frame: &mut Frame, area: Rect, app: &App) {
    let content_height = LOGO.len() as u16 + 8;
    let v_pad = area.height.saturating_sub(content_height) / 2;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(v_pad),
            Constraint::Length(LOGO.len() as u16 + 2),
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(area);

    // Logo - truncate lines if terminal is narrow
    let logo_lines: Vec<Line> = LOGO
        .iter()
        .map(|line| {
            let display: String = line.chars().take(area.width as usize).collect();
            Line::from(Span::styled(display, Style::default().fg(Color::Cyan)))
        })
        .collect();
    let logo = Paragraph::new(logo_lines).alignment(Alignment::Center);
    frame.render_widget(logo, chunks[1]);

    // Subtitle
    let subtitle = Paragraph::new(Line::from(vec![
        Span::styled(
            "Claude Code Usage Analytics",
            Style::default().fg(Color::White).bold(),
        ),
    ]))
    .alignment(Alignment::Center);
    frame.render_widget(subtitle, chunks[2]);

    // Blinking prompt
    let blink = if app.welcome_frame % 20 < 14 {
        "Press Enter to continue"
    } else {
        ""
    };
    let prompt = Paragraph::new(Line::from(Span::styled(
        blink,
        Style::default().fg(Color::DarkGray).italic(),
    )))
    .alignment(Alignment::Center);
    frame.render_widget(prompt, chunks[3]);
}

fn render_main_menu(frame: &mut Frame, area: Rect, app: &App) {
    let is_wide = area.width >= 80;

    if is_wide {
        // Two-column layout: menu on left, description panel on right
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .margin(1)
            .split(area);

        render_menu_list(frame, columns[0], app);
        render_menu_detail_panel(frame, columns[1], app);
    } else {
        // Single column: just the menu
        let inner = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0)])
            .margin(1)
            .split(area);
        render_menu_list(frame, inner[0], app);
    }
}

fn render_menu_list(frame: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = MAIN_MENU_ITEMS
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let style = if i == app.menu_index {
                Style::default().fg(Color::Cyan).bold()
            } else {
                Style::default().fg(Color::White)
            };
            let prefix = if i == app.menu_index { " > " } else { "   " };
            ListItem::new(Line::from(Span::styled(
                format!("{prefix}{label}"),
                style,
            )))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(Span::styled(
                    " Main Menu ",
                    Style::default().fg(Color::Cyan).bold(),
                ))
                .padding(Padding::new(1, 1, 1, 1)),
        )
        .highlight_style(Style::default());
    frame.render_widget(list, area);
}

fn render_menu_detail_panel(frame: &mut Frame, area: Rect, app: &App) {
    let desc = MAIN_MENU_DESCRIPTIONS
        .get(app.menu_index)
        .unwrap_or(&"");

    let content = vec![
        Line::from(""),
        Line::from(Span::styled(
            MAIN_MENU_ITEMS[app.menu_index],
            Style::default().fg(Color::Cyan).bold(),
        )),
        Line::from(""),
        Line::from(Span::styled(*desc, Style::default().fg(Color::White))),
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled(
            format!("  Status: {}", app.config_status),
            Style::default().fg(if app.config_status == "Connected" {
                Color::Green
            } else {
                Color::Yellow
            }),
        )),
        Line::from(Span::styled(
            format!("  Pending events: {}", app.metrics_lines),
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let panel = Paragraph::new(content).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(
                " Details ",
                Style::default().fg(Color::White),
            ))
            .padding(Padding::new(1, 1, 0, 0)),
    );
    frame.render_widget(panel, area);
}

fn render_submenu(frame: &mut Frame, area: Rect, app: &App, parent: usize) {
    let sub_items = match SUB_MENUS.get(parent) {
        Some(items) => items,
        None => return,
    };
    let icons = SUB_MENU_ICONS.get(parent).unwrap_or(&(&[] as &[&str]));
    let parent_label = MAIN_MENU_ITEMS[parent];

    let is_wide = area.width >= 80;

    if is_wide {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .margin(1)
            .split(area);

        render_submenu_list(frame, columns[0], app, sub_items, icons, parent_label);
        render_submenu_preview(frame, columns[1], app, parent, sub_items);
    } else {
        let inner = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0)])
            .margin(1)
            .split(area);
        render_submenu_list(frame, inner[0], app, sub_items, icons, parent_label);
    }
}

fn render_submenu_list(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    items: &[&str],
    icons: &[&str],
    parent_label: &str,
) {
    let mut list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let style = if i == app.sub_index {
                Style::default().fg(Color::Cyan).bold()
            } else {
                Style::default().fg(Color::White)
            };
            let prefix = if i == app.sub_index { " > " } else { "   " };
            let icon = icons.get(i).unwrap_or(&"  ");
            ListItem::new(Line::from(Span::styled(
                format!("{prefix}{icon}{label}"),
                style,
            )))
        })
        .collect();

    // Add "Back" item
    let back_style = if app.sub_index == items.len() {
        Style::default().fg(Color::Cyan).bold()
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let back_prefix = if app.sub_index == items.len() {
        " > "
    } else {
        "   "
    };
    list_items.push(ListItem::new(Line::from(Span::styled(
        format!("{back_prefix}  Back"),
        back_style,
    ))));

    let breadcrumb = format!(" {parent_label} ");
    let list = List::new(list_items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(breadcrumb, Style::default().fg(Color::Cyan).bold()))
            .padding(Padding::new(1, 1, 1, 1)),
    );
    frame.render_widget(list, area);
}

fn render_submenu_preview(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    parent: usize,
    items: &[&str],
) {
    let is_back = app.sub_index >= items.len();

    let content = if is_back {
        vec![
            Line::from(""),
            Line::from(Span::styled(
                "  Return to main menu",
                Style::default().fg(Color::DarkGray),
            )),
        ]
    } else {
        let item_name = items.get(app.sub_index).unwrap_or(&"");
        let placeholder_content = get_placeholder_content(parent, app.sub_index);

        let mut lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("  {item_name}"),
                Style::default().fg(Color::Cyan).bold(),
            )),
            Line::from(""),
        ];
        for line in placeholder_content {
            lines.push(Line::from(Span::styled(
                format!("  {line}"),
                Style::default().fg(Color::White),
            )));
        }
        lines
    };

    let panel = Paragraph::new(content)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(Span::styled(" Preview ", Style::default().fg(Color::White)))
                .padding(Padding::new(1, 1, 0, 0)),
        );
    frame.render_widget(panel, area);
}

fn render_detail(frame: &mut Frame, area: Rect, parent: usize, item: usize) {
    let parent_label = MAIN_MENU_ITEMS.get(parent).unwrap_or(&"");
    let item_label = SUB_MENUS
        .get(parent)
        .and_then(|m| m.get(item))
        .unwrap_or(&"");

    let placeholder = get_placeholder_content(parent, item);

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {parent_label} > {item_label}"),
            Style::default().fg(Color::Cyan).bold(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  This is a POC detail view.",
            Style::default().fg(Color::DarkGray).italic(),
        )),
        Line::from(Span::styled(
            "  In a full implementation, this would show live data.",
            Style::default().fg(Color::DarkGray).italic(),
        )),
        Line::from(""),
    ];
    for line in &placeholder {
        lines.push(Line::from(Span::styled(
            format!("  {line}"),
            Style::default().fg(Color::White),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Press Esc or Enter to go back",
        Style::default().fg(Color::DarkGray),
    )));

    let panel = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(Span::styled(
                    format!(" {item_label} "),
                    Style::default().fg(Color::Cyan).bold(),
                ))
                .padding(Padding::new(1, 1, 0, 0)),
        );

    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0)])
        .margin(1)
        .split(area);

    frame.render_widget(panel, inner[0]);
}

fn get_placeholder_content(parent: usize, item: usize) -> Vec<&'static str> {
    match (parent, item) {
        (0, 0) => vec![
            "Total sessions today:     12",
            "Total prompts:           847",
            "Most active project:     claudnalytics",
            "Avg session duration:    34m",
        ],
        (0, 1) => vec![
            "Mon  ████████████  142",
            "Tue  ██████████    128",
            "Wed  ████████████████  187",
            "Thu  ██████████████  156",
            "Fri  ████████      104",
        ],
        (0, 2) => vec![
            "This week:   717 prompts across 42 sessions",
            "Last week:   634 prompts across 38 sessions",
            "Change:      +13.1%",
        ],
        (0, 3) => vec![
            "Export formats:  JSON, CSV",
            "Date range:      Last 30 days",
            "Includes:        Sessions, tools, tokens",
        ],
        (1, 0) => vec![
            "Currently active: 1 session",
            "Project: claudnalytics/native",
            "Duration: 12m 34s",
        ],
        (1, 1) => vec![
            "Last 24h:   8 sessions",
            "Last 7d:   42 sessions",
            "Last 30d: 156 sessions",
        ],
        (1, 2) => vec![
            "Select a session to view:",
            "  Prompts, tool calls, token usage,",
            "  duration, and permission requests",
        ],
        (2, 0) => vec![
            "claudnalytics    ████████████████  312",
            "webgate-api      ██████████        201",
            "frontend         ████████          168",
        ],
        (2, 1) => vec![
            "Compare metrics across projects:",
            "  Prompts, sessions, tool usage,",
            "  and token consumption",
        ],
        (2, 2) => vec![
            "     Mon Tue Wed Thu Fri Sat Sun",
            "W1    .   .  ##  ##   .   .   . ",
            "W2   ##  ##  ##  ##  ##   .   . ",
            "W3    .  ##  ##   .  ##   .   . ",
            "W4   ##  ##  ##  ##   .   .   . ",
        ],
        (3, 0) => vec![
            "Read      ████████████████  892",
            "Edit      ████████████      634",
            "Bash      ██████████        487",
            "Write     ██████            312",
            "Glob      ████              198",
        ],
        (3, 1) => vec![
            "Avg tool latency:   1.2s",
            "Slowest tool:       Bash (3.4s avg)",
            "Fastest tool:       Glob (0.1s avg)",
        ],
        (3, 2) => vec![
            "Total requests:  47",
            "Auto-approved:   12",
            "Most requested:  Bash (execute)",
        ],
        (4, 0) => vec![
            "API Base:   http://localhost:3001/api",
            "Auto-sync:  Enabled (on boundary events)",
            "Buffer:     10 events",
        ],
        (4, 1) => vec![
            "Current key:  clk_654f...6a19",
            "Created:      2026-02-14",
            "Last sync:    2 minutes ago",
        ],
        (4, 2) => vec![
            "Theme:        Dark (default)",
            "Refresh rate: 100ms",
            "Vim keys:     Enabled",
        ],
        _ => vec!["No preview available"],
    }
}

// ---- Terminal setup ----

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) {
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
}

// ---- Main loop ----

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

        let _ = terminal.draw(|frame| {
            let size = frame.area();
            let layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(2),  // header
                    Constraint::Min(10),   // content
                    Constraint::Length(2),  // footer
                ])
                .split(size);

            render_header(frame, layout[0]);

            match &app.screen {
                Screen::Welcome => render_welcome(frame, layout[1], &app),
                Screen::MainMenu => render_main_menu(frame, layout[1], &app),
                Screen::SubMenu(parent) => render_submenu(frame, layout[1], &app, *parent),
                Screen::Detail(parent, item) => render_detail(frame, layout[1], *parent, *item),
            }

            render_footer(frame, layout[2], &app.screen);
        });

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout).unwrap_or(false) {
            if let Ok(Event::Key(key)) = event::read() {
                if key.kind == KeyEventKind::Press {
                    app.handle_key(key.code);
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            app.welcome_frame += 1;
            last_tick = Instant::now();
        }
    }

    restore_terminal(&mut terminal);
    0
}
