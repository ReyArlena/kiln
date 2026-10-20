//! Widgets shared by several screens.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::config::StatusBarConfig;
use crate::engine::Sample;
use crate::engine::util::format_bytes;

use super::input::{Hit, HitMap};
use super::theme;

/// The always-visible sensor bar. Uses the latest sample from the global sampler.
/// Numbers are padded to a fixed width so the bar does not jitter as values change.
pub fn draw_status_bar(frame: &mut Frame, area: Rect, sample: Option<&Sample>, config: &StatusBarConfig, temp_cap: Option<f32>, is_root: bool, mem_total: u64) {
    let mut spans: Vec<Span> = vec![Span::styled(format!(" kiln {} ", env!("CARGO_PKG_VERSION")), theme::title())];
    if !is_root {
        spans.push(Span::styled(" not root ", theme::badge_bad()));
        spans.push(Span::raw(" "));
    }
    let sep = || Span::styled("| ", theme::dim());
    let number = |text: String| Span::styled(text, theme::number());
    if let Some(s) = sample {
        if config.cpu_temp && let Some(t) = s.primary_temp_c {
            spans.push(sep());
            spans.push(Span::raw("CPU "));
            spans.push(Span::styled(format!("{:>3}C", t.round() as i32), theme::temp(t, temp_cap)));
        }
        if config.cpu_usage && let Some(p) = s.cpu_percent {
            spans.push(Span::raw(" "));
            spans.push(number(format!("{:>3}%", p.round() as i32)));
        }
        if config.cpu_freq && let Some(f) = s.freq_mhz {
            spans.push(Span::raw(" "));
            spans.push(number(format!("{:>4.1}GHz", f as f32 / 1000.0)));
        }
        if config.cpu_temp || config.cpu_usage || config.cpu_freq {
            spans.push(Span::raw(" "));
        }
        if config.memory && mem_total > 0 {
            spans.push(sep());
            spans.push(Span::raw("MEM "));
            spans.push(number(format!("{:>3}%", (s.mem_used_bytes as f64 * 100.0 / mem_total as f64).round() as i32)));
            spans.push(Span::raw(" ("));
            spans.push(number(format!("{:>6}", format_bytes(s.mem_used_bytes))));
            spans.push(Span::raw(") "));
        }
        if config.nvme_temp {
            for (id, t) in s.temps.iter().filter(|(id, _)| id.starts_with("nvme") && id.ends_with("/Composite")) {
                spans.push(sep());
                spans.push(Span::raw(format!("{} ", id.split('/').next().unwrap_or("nvme").to_ascii_uppercase())));
                spans.push(Span::styled(format!("{:>3}C ", t.round() as i32), theme::temp(*t, Some(80.0))));
            }
        }
        if config.gpu_temp {
            for (_, t) in s.temps.iter().filter(|(id, _)| id == "amdgpu/edge" || id == "nouveau/temp1") {
                spans.push(sep());
                spans.push(Span::raw("GPU "));
                spans.push(Span::styled(format!("{:>3}C ", t.round() as i32), theme::temp(*t, Some(95.0))));
            }
        }
        for extra in &config.extra_sensors {
            if let Some((_, t)) = s.temps.iter().find(|(id, _)| id == extra) {
                spans.push(sep());
                spans.push(Span::raw(format!("{extra} ")));
                spans.push(Span::styled(format!("{:>3}C ", t.round() as i32), theme::temp(*t, None)));
            }
        }
    } else {
        spans.push(Span::styled("| sensors warming up", theme::dim()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).style(Style::default().bg(theme::BAR_BG)), area);
}

/// Bottom line of key hints. Each hint is clickable and acts like its key.
pub fn draw_key_hints(frame: &mut Frame, area: Rect, hints: &[(&str, &str)], hits: &mut HitMap) {
    let mut spans: Vec<Span> = Vec::new();
    let mut x = area.x;
    for (key, label) in hints {
        let text = format!(" {key} ");
        let width = (text.len() + label.len() + 1) as u16;
        if let Some(c) = hint_char(key) {
            hits.add(Rect::new(x, area.y, width, 1), Hit::Button(c));
        }
        spans.push(Span::styled(text, theme::key()));
        spans.push(Span::raw(format!("{label} ")));
        x += width;
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Map a hint label like `Enter`, `Space`, `q` to the character its click should send.
fn hint_char(key: &str) -> Option<char> {
    match key {
        "Enter" => Some('\n'),
        "Space" => Some(' '),
        "Esc" => Some('\x1b'),
        "Tab" => Some('\t'),
        k if k.chars().count() == 1 => k.chars().next(),
        _ => None,
    }
}

/// A bordered block whose border colour shows focus.
pub fn pane_block(title: &str, focused: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(if focused {
            theme::focused_border()
        } else {
            theme::border()
        })
        .title(Span::styled(
            format!(" {title} "),
            if focused {
                theme::title()
            } else {
                Style::default()
            },
        ))
}

/// Centre a box of the given size inside `area`.
pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [v] = Layout::vertical([Constraint::Length(height.min(area.height))])
        .flex(Flex::Center)
        .areas(area);
    let [h] = Layout::horizontal([Constraint::Length(width.min(area.width))])
        .flex(Flex::Center)
        .areas(v);
    h
}

/// What a modal dialog is asking.
#[derive(Debug, Clone, PartialEq)]
pub enum ModalKind {
    /// Just a message. Enter or Esc closes it.
    Message,
    /// Yes/No. Enter or `y` confirms, Esc or `n` cancels.
    Confirm,
    /// Must type `expected` exactly then press Enter.
    Typed { expected: String, input: TextInput },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Modal {
    pub title: String,
    pub body: String,
    pub kind: ModalKind,
    /// Screen-specific tag saying what to do on confirm.
    pub tag: String,
}

impl Modal {
    pub fn message(title: &str, body: &str) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            kind: ModalKind::Message,
            tag: String::new(),
        }
    }

    pub fn confirm(title: &str, body: &str, tag: &str) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            kind: ModalKind::Confirm,
            tag: tag.into(),
        }
    }

    pub fn typed(title: &str, body: &str, expected: &str, tag: &str) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            kind: ModalKind::Typed {
                expected: expected.into(),
                input: TextInput::default(),
            },
            tag: tag.into(),
        }
    }

    /// Feed a key. Returns `Some(true)` on confirm, `Some(false)` on cancel, `None` to stay open.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<bool> {
        match &mut self.kind {
            ModalKind::Message => match key.code {
                KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char(' ') => {
                    Some(true)
                }
                _ => None,
            },
            ModalKind::Confirm => match key.code {
                KeyCode::Enter | KeyCode::Char('y') => Some(true),
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => Some(false),
                _ => None,
            },
            ModalKind::Typed { expected, input } => match key.code {
                KeyCode::Esc => Some(false),
                KeyCode::Enter => (input.value == *expected).then_some(true),
                _ => {
                    input.handle_key(key);
                    None
                }
            },
        }
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        let width = (self
            .body
            .lines()
            .map(|l| l.len())
            .max()
            .unwrap_or(20)
            .max(self.title.len()) as u16
            + 6)
        .max(30)
        .min(area.width.saturating_sub(4).max(1));
        let body_lines = self.body.lines().count() as u16 + 2;
        let extra = match self.kind {
            ModalKind::Message => 2,
            ModalKind::Confirm => 2,
            ModalKind::Typed { .. } => 4,
        };
        let rect = centered(area, width, body_lines + extra + 2);
        frame.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme::focused_border())
            .title(Span::styled(format!(" {} ", self.title), theme::title()));
        let inner = block.inner(rect);
        frame.render_widget(block, rect);

        let mut lines: Vec<Line> = self
            .body
            .lines()
            .map(|l| Line::from(l.to_string()))
            .collect();
        lines.push(Line::from(""));
        match &self.kind {
            ModalKind::Message => lines.push(Line::from(vec![
                Span::styled("Enter", theme::key()),
                Span::raw(" close"),
            ])),
            ModalKind::Confirm => lines.push(Line::from(vec![
                Span::styled("y", theme::key()),
                Span::raw(" / "),
                Span::styled("Enter", theme::key()),
                Span::raw(" yes   "),
                Span::styled("n", theme::key()),
                Span::raw(" / "),
                Span::styled("Esc", theme::key()),
                Span::raw(" no"),
            ])),
            ModalKind::Typed { expected, input } => {
                lines.push(Line::from(vec![
                    Span::raw("Type "),
                    Span::styled(expected.clone(), Style::default().bold()),
                    Span::raw(" to confirm:"),
                ]));
                lines.push(input.line(true));
                lines.push(Line::from(vec![
                    Span::styled("Enter", theme::key()),
                    Span::raw(" confirm   "),
                    Span::styled("Esc", theme::key()),
                    Span::raw(" cancel"),
                ]));
            }
        }
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }
}

/// Minimal single-line text editor.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextInput {
    pub value: String,
    pub cursor: usize,
}

impl TextInput {
    pub fn with_value(value: &str) -> Self {
        Self {
            value: value.to_string(),
            cursor: value.chars().count(),
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(c) => {
                let idx = self.byte_index();
                self.value.insert(idx, c);
                self.cursor += 1;
            }
            KeyCode::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                let idx = self.byte_index();
                self.value.remove(idx);
            }
            KeyCode::Delete if self.cursor < self.value.chars().count() => {
                let idx = self.byte_index();
                self.value.remove(idx);
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.value.chars().count()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.value.chars().count(),
            _ => {}
        }
    }

    fn byte_index(&self) -> usize {
        self.value
            .char_indices()
            .nth(self.cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.value.len())
    }

    /// Render with a visible cursor block.
    pub fn line(&self, active: bool) -> Line<'static> {
        if !active {
            return Line::from(self.value.clone());
        }
        let chars: Vec<char> = self.value.chars().collect();
        let before: String = chars[..self.cursor.min(chars.len())].iter().collect();
        let at: String = chars
            .get(self.cursor)
            .map(|c| c.to_string())
            .unwrap_or_else(|| " ".into());
        let after: String = chars
            .get(self.cursor + 1..)
            .map(|c| c.iter().collect())
            .unwrap_or_default();
        Line::from(vec![
            Span::raw(before),
            Span::styled(at, Style::default().reversed()),
            Span::raw(after),
        ])
    }
}

/// Full-screen help overlay.
pub fn draw_help(frame: &mut Frame, area: Rect, screen_keys: &[(&str, &str)]) {
    let mut lines = vec![
        Line::from(Span::styled("Navigation (reserved keys)", theme::title())),
        Line::from("  Arrows / h j k l   move, scroll"),
        Line::from("  PgUp PgDn Home End  jump"),
        Line::from("  Tab / Shift-Tab     next / previous pane"),
        Line::from("  Mouse               click to select or toggle, wheel to scroll"),
        Line::from(""),
        Line::from(Span::styled("Everywhere", theme::title())),
        Line::from("  Space   toggle      Enter   confirm / open"),
        Line::from("  Esc     back        q       quit"),
        Line::from("  ?       this help"),
        Line::from(""),
        Line::from(Span::styled("This screen", theme::title())),
    ];
    for (k, label) in screen_keys {
        lines.push(Line::from(format!("  {k:<7} {label}")));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press any key to close",
        theme::dim(),
    )));
    let rect = centered(area, 64, lines.len() as u16 + 2);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::focused_border())
        .title(Span::styled(" Help ", theme::title()))
        .title_alignment(Alignment::Center);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Keep a list cursor inside bounds and compute a scroll offset that keeps it visible.
pub fn clamp_scroll(selected: usize, len: usize, visible: usize, offset: usize) -> usize {
    if len == 0 || visible == 0 {
        return 0;
    }
    let selected = selected.min(len - 1);
    if selected < offset {
        selected
    } else if selected >= offset + visible {
        selected + 1 - visible
    } else {
        offset.min(len.saturating_sub(visible))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    #[test]
    fn text_input_edits_at_cursor() {
        let mut input = TextInput::with_value("abc");
        input.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        input.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE));
        assert_eq!(input.value, "abXc");
        input.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(input.value, "abc");
    }

    #[test]
    fn typed_modal_only_confirms_on_exact_match() {
        let mut modal = Modal::typed("Wipe", "really?", "/dev/sdz", "wipe");
        for c in "/dev/sdz".chars() {
            assert_eq!(
                modal.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)),
                None
            );
        }
        assert_eq!(
            modal.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Some(true)
        );
        let mut wrong = Modal::typed("Wipe", "really?", "/dev/sdz", "wipe");
        assert_eq!(
            wrong.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            wrong.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Some(false)
        );
    }

    #[test]
    fn scroll_offset_follows_selection() {
        assert_eq!(clamp_scroll(0, 10, 5, 0), 0);
        assert_eq!(clamp_scroll(7, 10, 5, 0), 3);
        assert_eq!(clamp_scroll(2, 10, 5, 4), 2);
        assert_eq!(clamp_scroll(9, 10, 5, 9), 5);
    }
}
