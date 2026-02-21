#![allow(dead_code)]

use ratatui::style::{Color, Modifier, Style};

pub const ACCENT: Color = Color::Rgb(204, 138, 101);
pub const TEXT: Color = Color::Rgb(224, 224, 224);
pub const TEXT_DIM: Color = Color::DarkGray;
pub const BORDER: Color = Color::Rgb(68, 68, 68);
pub const SUCCESS: Color = Color::Rgb(107, 191, 107);
pub const WARNING: Color = Color::Rgb(224, 192, 80);
pub const ERROR: Color = Color::Rgb(224, 96, 96);

pub fn accent() -> Style {
    Style::default().fg(ACCENT)
}

pub fn accent_bold() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

pub fn text() -> Style {
    Style::default().fg(TEXT)
}

pub fn dim() -> Style {
    Style::default().fg(TEXT_DIM)
}

pub fn border() -> Style {
    Style::default().fg(BORDER)
}

pub fn border_focused() -> Style {
    Style::default().fg(ACCENT)
}

pub fn success() -> Style {
    Style::default().fg(SUCCESS)
}

pub fn warning() -> Style {
    Style::default().fg(WARNING)
}
