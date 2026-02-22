/// vibenalytics v3 — Claude Code usage analytics CLI
///
/// Single Rust binary. Zero runtime dependencies.

mod aggregation;
mod auth;
mod config;
mod hash;
mod http;
mod import;
mod log_cmd;
mod paths;
mod projects;
mod sync;
mod transcripts;
mod tui;
mod update;

use clap::{Parser, Subcommand};
use std::env;
use std::io::IsTerminal;

#[derive(Parser)]
#[command(
    name = "vibenalytics",
    about = "Claude Code usage analytics",
    version,
    after_help = "EXAMPLES:\n    vibenalytics                    Launch the dashboard\n    vibenalytics init               First-time setup\n    vibenalytics project list       See tracked projects\n    vibenalytics import --dry       Preview history import\n\nDOCS:\n    https://docs.vibenalytics.dev"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Verbose output (repeat for more: -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// Suppress non-error output
    #[arg(short, long, global = true)]
    quiet: bool,

    /// Disable colored output
    #[arg(long, global = true)]
    no_color: bool,

    /// Machine-readable JSON output
    #[arg(long, global = true)]
    json: bool,

}

#[derive(Subcommand)]
enum Commands {
    /// Set up Vibenalytics (scan projects, configure sync)
    Init,

    /// Authenticate via browser
    Login,

    /// Clear stored credentials
    Logout,

    /// Show connection status and sync health
    Status,

    /// Manage tracked projects
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },

    /// Manually trigger a sync
    Sync {
        /// Use transcript-based sync
        #[arg(long)]
        use_transcripts: bool,

        /// Force sync even if recently synced
        #[arg(long)]
        force: bool,
    },

    /// Import session history from ~/.claude/
    Import {
        /// Filter by project name (substring match)
        project: Option<String>,

        /// Parse only, skip backend sync
        #[arg(long)]
        dry: bool,
    },

    /// Update to the latest version
    Update,

    /// (internal) Hook handler — reads event JSON from stdin
    #[command(hide = true)]
    Log {
        /// Use transcript-based logging
        #[arg(long)]
        use_transcripts: bool,
    },
}

#[derive(Subcommand)]
enum ProjectAction {
    /// List tracked projects
    List,

    /// Add a project (defaults to current directory)
    Add {
        /// Path to project directory
        path: Option<String>,
    },

    /// Remove a project from tracking
    Remove {
        /// Project name (or omit to use current directory)
        name: Option<String>,
    },

    /// Re-enable a paused project
    Enable {
        /// Project name (or omit to use current directory)
        name: Option<String>,
    },

    /// Pause syncing without removing
    Disable {
        /// Project name (or omit to use current directory)
        name: Option<String>,
    },
}

fn resolve_name_or_cwd(name: Option<String>) -> String {
    name.unwrap_or_else(|| {
        env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".to_string())
    })
}

fn cmd_status(dir: &std::path::Path, json_output: bool) -> i32 {
    let cfg = match config::read_config(dir) {
        Some(c) => c,
        None => {
            if json_output {
                println!(r#"{{"status":"not_configured"}}"#);
            } else {
                println!("Not configured. Run: vibenalytics init");
            }
            return 1;
        }
    };

    let get = |k: &str| -> String {
        cfg.get(k)
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string()
    };

    let key = get("apiKey");
    let name = get("displayName");
    let registry = projects::read_projects(dir);
    let active = registry.projects.iter().filter(|p| p.enabled).count();
    let total = registry.projects.len();

    if json_output {
        let key_display = if key.len() > 12 {
            format!("{}...{}", &key[..8], &key[key.len() - 4..])
        } else {
            key
        };
        println!(
            r#"{{"status":"connected","name":"{}","api_key":"{}","projects_active":{},"projects_total":{}}}"#,
            name, key_display, active, total
        );
    } else {
        let key_display = if key.len() > 12 {
            format!("{}...{}", &key[..8], &key[key.len() - 4..])
        } else {
            key
        };
        println!("Configured:");
        println!("  Name:     {}", name);
        println!("  API Key:  {}", key_display);
        println!("  Projects: {} active / {} total", active, total);
    }
    0
}

fn cmd_project_list(dir: &std::path::Path, json_output: bool) -> i32 {
    let registry = projects::read_projects(dir);

    if json_output {
        let json = serde_json::to_string_pretty(&registry).unwrap_or_default();
        println!("{json}");
        return 0;
    }

    if registry.projects.is_empty() {
        println!("No projects tracked. Run: vibenalytics project add");
        return 0;
    }

    println!(
        "  {:<16} {:<10} {}",
        "NAME", "STATUS", "PATH"
    );
    for p in &registry.projects {
        let status = if p.enabled { "active" } else { "paused" };
        println!("  {:<16} {:<10} {}", p.name, status, p.path);
    }
    0
}

fn main() {
    let cli = Cli::parse();
    let dir = paths::data_dir();

    let rc = match cli.command {
        None => {
            // No subcommand: launch TUI if interactive, status if piped
            if std::io::stdout().is_terminal() {
                tui::run(&dir)
            } else {
                cmd_status(&dir, cli.json)
            }
        }

        Some(Commands::Init) => {
            // TODO: Implement onboarding wizard (spec section 3)
            eprintln!("TODO: Onboarding wizard not yet implemented.");
            eprintln!("For now, use: vibenalytics login && vibenalytics project add");
            1
        }

        Some(Commands::Login) => auth::cmd_login(&dir),
        Some(Commands::Logout) => auth::cmd_logout(&dir),
        Some(Commands::Status) => cmd_status(&dir, cli.json),

        Some(Commands::Project { action }) => match action {
            ProjectAction::List => cmd_project_list(&dir, cli.json),

            ProjectAction::Add { path } => {
                let p = path.unwrap_or_else(|| {
                    env::current_dir()
                        .map(|d| d.to_string_lossy().to_string())
                        .unwrap_or_else(|_| ".".to_string())
                });
                match projects::add_project(&dir, &p) {
                    Ok(name) => {
                        println!("Added project \"{}\"", name);
                        0
                    }
                    Err(msg) => {
                        eprintln!("{msg}");
                        1
                    }
                }
            }

            ProjectAction::Remove { name } => {
                let target = resolve_name_or_cwd(name);
                match projects::remove_project(&dir, &target) {
                    Ok(name) => {
                        println!("Removed \"{}\" from tracking.", name);
                        0
                    }
                    Err(msg) => {
                        eprintln!("{msg}");
                        1
                    }
                }
            }

            ProjectAction::Enable { name } => {
                let target = resolve_name_or_cwd(name);
                match projects::enable_project(&dir, &target) {
                    Ok(name) => {
                        println!("Resumed syncing for \"{}\".", name);
                        0
                    }
                    Err(msg) => {
                        eprintln!("{msg}");
                        1
                    }
                }
            }

            ProjectAction::Disable { name } => {
                let target = resolve_name_or_cwd(name);
                match projects::disable_project(&dir, &target) {
                    Ok(name) => {
                        println!("Paused syncing for \"{}\".", name);
                        0
                    }
                    Err(msg) => {
                        eprintln!("{msg}");
                        1
                    }
                }
            }
        },

        Some(Commands::Sync { use_transcripts, .. }) => {
            if use_transcripts {
                sync::cmd_sync_transcripts(&dir)
            } else {
                sync::cmd_sync(&dir)
            }
        }

        Some(Commands::Import { project, dry }) => {
            import::cmd_import(&dir, project.as_deref(), !dry)
        }

        Some(Commands::Update) => update::cmd_update(),

        Some(Commands::Log { use_transcripts }) => {
            if use_transcripts {
                log_cmd::cmd_log_transcripts(&dir)
            } else {
                log_cmd::cmd_log(&dir)
            }
        }
    };

    std::process::exit(rc);
}
