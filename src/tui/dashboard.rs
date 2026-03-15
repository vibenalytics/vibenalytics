use std::sync::mpsc;
use chrono::Local;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use super::theme;
use crate::config::DEFAULT_API_BASE;
use crate::http::http_get;

pub struct OverviewStats {
    pub total_sessions: u32,
    pub total_prompts: u32,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cost: String,
    pub current_streak: u32,
    pub active_days: u32,
}

pub enum LoadState {
    Idle,
    Loading { rx: mpsc::Receiver<Result<OverviewStats, String>> },
    Loaded,
    Error(String),
}

pub struct DashboardState {
    pub today: Option<OverviewStats>,
    pub load_state: LoadState,
}

impl Default for DashboardState {
    fn default() -> Self {
        DashboardState {
            today: None,
            load_state: LoadState::Idle,
        }
    }
}

impl DashboardState {
    pub fn poll(&mut self) {
        let next = if let LoadState::Loading { rx } = &self.load_state {
            match rx.try_recv() {
                Ok(Ok(stats)) => {
                    self.today = Some(stats);
                    Some(LoadState::Loaded)
                }
                Ok(Err(e)) => Some(LoadState::Error(e)),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(LoadState::Error("Connection lost".into()))
                }
            }
        } else {
            None
        };
        if let Some(state) = next {
            self.load_state = state;
        }
    }
}

pub fn start_loading(api_key: &str) -> mpsc::Receiver<Result<OverviewStats, String>> {
    let (tx, rx) = mpsc::channel();
    let api_base = DEFAULT_API_BASE.to_string();
    let key = api_key.to_string();

    std::thread::spawn(move || {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let url = format!("{api_base}/stats/overview?from={today}&to={today}");
        let result = fetch_stats(&url, &key);
        let _ = tx.send(result);
    });

    rx
}

fn fetch_stats(url: &str, api_key: &str) -> Result<OverviewStats, String> {
    let (status, body) = http_get(url, Some(api_key))?;
    if status != 200 {
        return Err(format!("HTTP {status}"));
    }

    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("Invalid JSON: {e}"))?;

    let ok = json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    if !ok {
        let err = json.get("error").and_then(|v| v.as_str()).unwrap_or("Unknown error");
        return Err(err.to_string());
    }

    let data = json.get("data").ok_or("Missing data field")?;

    Ok(OverviewStats {
        total_sessions: data.get("totalSessions").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        total_prompts: data.get("totalPrompts").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        total_input_tokens: parse_token_str(data.get("totalInputTokens")),
        total_output_tokens: parse_token_str(data.get("totalOutputTokens")),
        total_cost: data.get("totalCost").and_then(|v| v.as_str()).unwrap_or("0.00").to_string(),
        current_streak: data.get("currentStreak").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        active_days: data.get("activeDays").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
    })
}

fn parse_token_str(val: Option<&serde_json::Value>) -> u64 {
    match val {
        Some(serde_json::Value::String(s)) => s.parse().unwrap_or(0),
        Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(0),
        _ => 0,
    }
}

fn format_tokens(count: u64) -> String {
    if count == 0 {
        "0".to_string()
    } else if count < 1_000 {
        format!("{count}")
    } else if count < 1_000_000 {
        let k = count as f64 / 1_000.0;
        if k >= 100.0 {
            format!("{:.0}K", k)
        } else if k >= 10.0 {
            format!("{:.0}K", k)
        } else {
            format!("{:.1}K", k)
        }
    } else {
        let m = count as f64 / 1_000_000.0;
        if m >= 10.0 {
            format!("{:.0}M", m)
        } else {
            format!("{:.1}M", m)
        }
    }
}

pub fn render(frame: &mut Frame, area: Rect, state: &DashboardState) {
    let mut lines = vec![Line::from("")];

    match &state.load_state {
        LoadState::Idle => {
            lines.push(Line::from(Span::styled("  Log in to see your stats.", theme::dim())));
        }
        LoadState::Loading { .. } => {
            let spinner = theme::spinner_char();
            lines.push(Line::from(vec![
                Span::styled(format!("  {spinner} "), theme::accent_bold()),
                Span::styled("Loading stats...", theme::dim()),
            ]));
        }
        LoadState::Error(e) => {
            lines.push(Line::from(Span::styled(format!("  Failed to load stats: {e}"), theme::dim())));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("  Run 'vibenalytics sync' to push local data first.", theme::dim())));
        }
        LoadState::Loaded => {
            if let Some(stats) = &state.today {
                lines.push(Line::from(Span::styled("  Today", theme::accent_bold())));
                lines.push(Line::from(""));

                for (label, value, style) in [
                    ("Sessions", format!("{}", stats.total_sessions), theme::text()),
                    ("Prompts", format!("{}", stats.total_prompts), theme::text()),
                    ("Tokens in", format_tokens(stats.total_input_tokens), theme::text()),
                    ("Tokens out", format_tokens(stats.total_output_tokens), theme::text()),
                    ("Cost", format!("${}", stats.total_cost), theme::accent_bold()),
                ] {
                    lines.push(Line::from(vec![
                        Span::styled(format!("    {:<15}", label), theme::dim()),
                        Span::styled(value, style),
                    ]));
                }

                if stats.current_streak > 0 {
                    lines.push(Line::from(""));
                    let streak_label = if stats.current_streak == 1 { "day" } else { "days" };
                    lines.push(Line::from(vec![
                        Span::styled(format!("    {:<15}", "Streak"), theme::dim()),
                        Span::styled(format!("{} {streak_label}", stats.current_streak), theme::success()),
                    ]));
                }
            } else {
                lines.push(Line::from(Span::styled("  No data for today.", theme::dim())));
            }
        }
    }

    frame.render_widget(Paragraph::new(lines), area);
}
