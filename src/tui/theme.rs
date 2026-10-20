//! Colours and styles in one place.

use ratatui::style::{Color, Modifier, Style};

pub const ACCENT: Color = Color::Rgb(255, 140, 60);
pub const DIM: Color = Color::DarkGray;
pub const OK: Color = Color::Green;
pub const WARN: Color = Color::Yellow;
pub const BAD: Color = Color::Red;
pub const INFO: Color = Color::Cyan;
pub const BAR_BG: Color = Color::Rgb(30, 30, 36);

pub fn title() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

pub fn selected() -> Style {
    Style::default()
        .bg(Color::Rgb(60, 60, 70))
        .add_modifier(Modifier::BOLD)
}

pub fn focused_border() -> Style {
    Style::default().fg(ACCENT)
}

pub fn border() -> Style {
    Style::default().fg(DIM)
}

pub fn dim() -> Style {
    Style::default().fg(DIM)
}

/// Numeric values in the status bar and tables.
pub fn number() -> Style {
    Style::default().fg(INFO)
}

/// A red label block for warnings that must not be missed.
pub fn badge_bad() -> Style {
    Style::default().fg(Color::White).bg(BAD).add_modifier(Modifier::BOLD)
}

/// Secondary text that still has to be readable (field help, hints).
pub fn help() -> Style {
    Style::default().fg(Color::Gray)
}

pub fn key() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

pub fn status(label: &str) -> Style {
    match label {
        "PASS" => Style::default().fg(OK).add_modifier(Modifier::BOLD),
        "FAIL" => Style::default().fg(BAD).add_modifier(Modifier::BOLD),
        "ABORT" => Style::default().fg(WARN).add_modifier(Modifier::BOLD),
        "SKIP" => Style::default().fg(DIM),
        "RUNNING" => Style::default().fg(INFO).add_modifier(Modifier::BOLD),
        _ => Style::default(),
    }
}

/// Colour a temperature relative to the configured cap.
pub fn temp(temp_c: f32, cap_c: Option<f32>) -> Style {
    let cap = cap_c.unwrap_or(90.0);
    if temp_c >= cap - 2.0 {
        Style::default().fg(BAD).add_modifier(Modifier::BOLD)
    } else if temp_c >= cap - 10.0 {
        Style::default().fg(WARN)
    } else {
        Style::default().fg(OK)
    }
}
