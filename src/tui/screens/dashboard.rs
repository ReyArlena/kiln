//! Live view of a run in progress.

use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Gauge, Paragraph, Sparkline, Wrap};

use crate::engine::util::format_duration_secs;
use crate::engine::{Event, Plan, Status};

use super::super::input::{Action, Hit, HitMap};
use super::super::theme;
use super::super::widgets::{Modal, draw_key_hints, pane_block};
use super::Effect;

const PANE_LOG: usize = 0;
const PANE_DMESG: usize = 1;
const MAX_LINES: usize = 3000;
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
/// Bottom of the temperature sparkline, in Celsius.
const SPARK_FLOOR_C: u64 = 30;

struct TestRow {
    name: String,
    status: String,
    detail: String,
}

pub struct DashboardScreen {
    tests: Vec<TestRow>,
    current: Option<usize>,
    current_name: String,
    started: Instant,
    expected_secs: Option<u64>,
    temps: Vec<u64>,
    latest_temp: Option<f32>,
    temp_cap: Option<f32>,
    metrics: BTreeMap<String, f64>,
    log: VecDeque<String>,
    dmesg: VecDeque<String>,
    cooldown_left: Option<u64>,
    trip: Option<String>,
    focus: usize,
    /// Lines scrolled up from the bottom; 0 means follow the tail.
    log_scroll: usize,
    dmesg_scroll: usize,
    finished: bool,
    /// Redraw counter, drives the spinner.
    tick: usize,
}

impl DashboardScreen {
    pub fn empty() -> Self {
        Self::new(&Plan::new("", Default::default()))
    }

    pub fn new(plan: &Plan) -> Self {
        let mut tests = Vec::new();
        for entry in &plan.tests {
            for _ in 0..entry.repeat.max(1) {
                tests.push(TestRow {
                    name: entry.test.clone(),
                    status: "wait".into(),
                    detail: String::new(),
                });
            }
        }
        Self {
            tests,
            current: None,
            current_name: String::new(),
            started: Instant::now(),
            expected_secs: None,
            temps: Vec::new(),
            latest_temp: None,
            temp_cap: plan.guards.temp_max_c,
            metrics: BTreeMap::new(),
            log: VecDeque::new(),
            dmesg: VecDeque::new(),
            cooldown_left: None,
            trip: None,
            focus: PANE_LOG,
            log_scroll: 0,
            dmesg_scroll: 0,
            finished: false,
            tick: 0,
        }
    }

    pub fn key_hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.finished {
            vec![("Enter", "results")]
        } else {
            vec![
                ("c", "stop test"),
                ("C", "stop run"),
                ("Tab", "switch log"),
                ("?", "help"),
            ]
        }
    }

    /// Fold a runner event into the display.
    pub fn apply(&mut self, event: &Event) {
        match event {
            Event::TestStarted {
                index,
                name,
                expected_secs,
                ..
            } => {
                self.current = Some(*index);
                self.current_name = name.clone();
                self.started = Instant::now();
                self.expected_secs = *expected_secs;
                self.metrics.clear();
                self.trip = None;
                self.cooldown_left = None;
                if let Some(row) = self.tests.get_mut(*index) {
                    row.name = name.clone();
                    row.status = "RUNNING".into();
                }
                self.push_log(format!("=== {name}"));
            }
            Event::Line { text, .. } => self.push_log(text.clone()),
            Event::Metric { key, value, .. } => {
                self.metrics.insert(key.clone(), *value);
            }
            Event::Dmesg { text } => {
                if self.dmesg.len() >= MAX_LINES {
                    self.dmesg.pop_front();
                }
                self.dmesg.push_back(text.clone());
            }
            Event::GuardTripped { trip, .. } => self.trip = Some(trip.describe()),
            Event::Cooldown { secs_left } => self.cooldown_left = Some(*secs_left),
            Event::TestFinished { index, result } => {
                if let Some(row) = self.tests.get_mut(*index) {
                    row.status = result.status.label().into();
                    row.detail = result.status.detail();
                }
                let detail = result.status.detail();
                self.push_log(format!(
                    "=== {} {}{}",
                    result.name,
                    result.status.label(),
                    if detail.is_empty() {
                        String::new()
                    } else {
                        format!(": {detail}")
                    }
                ));
                if !matches!(result.status, Status::Pass) {
                    self.trip = None;
                }
            }
            Event::Sample(s) => {
                if let Some(t) = s.primary_temp_c {
                    self.latest_temp = Some(t);
                    self.temps.push(t.round().max(0.0) as u64);
                    if self.temps.len() > 600 {
                        self.temps.remove(0);
                    }
                }
            }
            Event::RunFinished(_) => self.finished = true,
        }
    }

    fn push_log(&mut self, text: String) {
        if self.log.len() >= MAX_LINES {
            self.log.pop_front();
        }
        self.log.push_back(text);
    }

    pub fn handle(&mut self, action: Action, hits: &HitMap) -> Vec<Effect> {
        let scroll = |value: &mut usize, len: usize, up: bool, step: usize| {
            *value = if up {
                (*value + step).min(len.saturating_sub(1))
            } else {
                value.saturating_sub(step)
            };
        };
        match action {
            Action::Up => {
                if self.focus == PANE_LOG {
                    scroll(&mut self.log_scroll, self.log.len(), true, 1)
                } else {
                    scroll(&mut self.dmesg_scroll, self.dmesg.len(), true, 1)
                }
            }
            Action::Down => {
                if self.focus == PANE_LOG {
                    scroll(&mut self.log_scroll, self.log.len(), false, 1)
                } else {
                    scroll(&mut self.dmesg_scroll, self.dmesg.len(), false, 1)
                }
            }
            Action::PageUp => {
                if self.focus == PANE_LOG {
                    scroll(&mut self.log_scroll, self.log.len(), true, 10)
                } else {
                    scroll(&mut self.dmesg_scroll, self.dmesg.len(), true, 10)
                }
            }
            Action::PageDown => {
                if self.focus == PANE_LOG {
                    scroll(&mut self.log_scroll, self.log.len(), false, 10)
                } else {
                    scroll(&mut self.dmesg_scroll, self.dmesg.len(), false, 10)
                }
            }
            Action::End => {
                self.log_scroll = 0;
                self.dmesg_scroll = 0;
            }
            Action::NextPane | Action::PrevPane | Action::Left | Action::Right => {
                self.focus = 1 - self.focus
            }
            Action::Char('c') if !self.finished => {
                return vec![Effect::Modal(Modal::confirm(
                    "Stop test",
                    "Stop the current test and move on?",
                    "cancel-test",
                ))];
            }
            Action::Char('C') if !self.finished => {
                return vec![Effect::Modal(Modal::confirm(
                    "Stop run",
                    "Stop the current test and the rest of the run?",
                    "cancel-run",
                ))];
            }
            Action::Quit if !self.finished => {
                return vec![Effect::Modal(Modal::confirm(
                    "Stop run",
                    "A run is in progress. Stop it and quit?",
                    "quit",
                ))];
            }
            Action::Click { x, y } => {
                if let Some(p) = hits.pane_at(x, y)
                    && (p == PANE_LOG || p == PANE_DMESG)
                {
                    self.focus = p;
                }
            }
            Action::Scroll { x, y, up } => match hits.pane_at(x, y) {
                Some(PANE_DMESG) => scroll(&mut self.dmesg_scroll, self.dmesg.len(), up, 3),
                _ => scroll(&mut self.log_scroll, self.log.len(), up, 3),
            },
            _ => {}
        }
        Vec::new()
    }

    /// Full-width state line: spinner, RUNNING/FINISHED, test name, progress.
    fn draw_banner(&mut self, frame: &mut Frame, area: Rect) {
        self.tick = self.tick.wrapping_add(1);
        let elapsed = format_duration_secs(self.started.elapsed().as_secs());
        let (label, style, detail) = if self.finished {
            let failed = self.tests.iter().filter(|t| t.status != "PASS").count();
            let summary = if failed == 0 {
                "all passed".to_string()
            } else {
                format!("{failed} not passed")
            };
            (
                "FINISHED".to_string(),
                theme::status(if failed == 0 { "PASS" } else { "FAIL" }),
                summary,
            )
        } else {
            let spinner = SPINNER[self.tick % SPINNER.len()];
            let progress = match (self.cooldown_left, self.expected_secs) {
                (Some(left), _) => format!("cooldown {left}s"),
                (None, Some(exp)) if exp > 0 => {
                    format!("{elapsed} / {}", format_duration_secs(exp))
                }
                _ => format!("{elapsed} elapsed"),
            };
            let position = self
                .current
                .map(|i| format!("{} of {}", i + 1, self.tests.len()))
                .unwrap_or_default();
            (
                format!("{spinner} RUNNING"),
                theme::status("RUNNING"),
                format!("{}   {position}   {progress}", self.current_name),
            )
        };
        let block = pane_block("", false).border_style(style);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let line = Line::from(vec![
            Span::styled(format!(" {label} "), style),
            Span::raw("  "),
            Span::raw(detail),
        ]);
        frame.render_widget(Paragraph::new(line), inner);
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, hits: &mut HitMap) {
        let [banner, body, hints] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .areas(area);
        self.draw_banner(frame, banner);
        let [top, bottom] =
            Layout::vertical([Constraint::Length(9), Constraint::Min(5)]).areas(body);
        let [tests_area, progress_area] =
            Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)]).areas(top);

        // Test list with statuses.
        let block = pane_block("Sequence", false);
        let inner = block.inner(tests_area);
        frame.render_widget(block, tests_area);
        let lines: Vec<Line> = self
            .tests
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let marker = if Some(i) == self.current && !self.finished {
                    ">"
                } else {
                    " "
                };
                Line::from(vec![
                    Span::raw(format!("{marker} {:<2} ", i + 1)),
                    Span::styled(format!("{:<8}", t.status), theme::status(&t.status)),
                    Span::raw(t.name.clone()),
                    Span::styled(
                        if t.detail.is_empty() {
                            String::new()
                        } else {
                            format!("  {}", t.detail)
                        },
                        theme::dim(),
                    ),
                ])
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);

        // Progress, temperature, metrics.
        let temp_title = match self.latest_temp {
            Some(t) => Line::from(vec![
                Span::raw(" CPU "),
                Span::styled(format!("{t:.1}C"), theme::temp(t, self.temp_cap)),
                Span::styled(
                    self.temp_cap
                        .map(|c| format!(" / cap {c:.0}C "))
                        .unwrap_or_else(|| " ".into()),
                    theme::dim(),
                ),
            ]),
            None => Line::from(Span::styled(" no sensor ", theme::dim())),
        };
        let block = pane_block(
            &if self.finished {
                "Finished".to_string()
            } else {
                format!("Now: {}", self.current_name)
            },
            false,
        )
        .title_top(temp_title.right_aligned());
        let inner = block.inner(progress_area);
        frame.render_widget(block, progress_area);
        let [gauge_area, temp_area, metrics_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Min(1),
        ])
        .areas(inner);

        let elapsed = self.started.elapsed().as_secs();
        let label = match (self.cooldown_left, self.expected_secs) {
            (Some(left), _) => format!("cooldown {left}s"),
            (None, Some(exp)) if exp > 0 => format!(
                "{} / {}",
                format_duration_secs(elapsed),
                format_duration_secs(exp)
            ),
            _ => format!("{} elapsed", format_duration_secs(elapsed)),
        };
        let ratio = match self.expected_secs {
            Some(exp) if exp > 0 && !self.finished => (elapsed as f64 / exp as f64).min(1.0),
            _ if self.finished => 1.0,
            _ => 0.0,
        };
        frame.render_widget(
            Gauge::default().ratio(ratio).label(label).gauge_style(
                Style::default()
                    .fg(theme::ACCENT)
                    .bg(ratatui::style::Color::Rgb(40, 40, 48)),
            ),
            gauge_area,
        );

        // Temperature reading lives in the pane's top border (right side), so the
        // sparkline gets the full width.
        let width = temp_area.width as usize;
        // Show 30C..cap rather than 0..cap so a few degrees of movement is visible,
        // and pad the left so the newest reading always sits at the right edge.
        let floor = SPARK_FLOOR_C;
        let ceiling = self
            .temp_cap
            .map(|c| c as u64)
            .unwrap_or(100)
            .max(floor + 10);
        let recent = self.temps.iter().rev().take(width).rev();
        let mut data: Vec<Option<u64>> =
            vec![None; width.saturating_sub(self.temps.len().min(width))];
        data.extend(recent.map(|t| Some(t.saturating_sub(floor).min(ceiling - floor))));
        frame.render_widget(
            Sparkline::default()
                .data(data)
                .max(ceiling - floor)
                .style(Style::default().fg(theme::ACCENT)),
            temp_area,
        );

        let metrics: Vec<String> = self
            .metrics
            .iter()
            .map(|(k, v)| {
                format!(
                    "{k}={}",
                    if v.fract() == 0.0 {
                        format!("{}", *v as i64)
                    } else {
                        format!("{v:.2}")
                    }
                )
            })
            .collect();
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(metrics.join("  "), theme::dim())))
                .wrap(Wrap { trim: true }),
            metrics_area,
        );

        // Logs.
        let [log_area, dmesg_area] =
            Layout::horizontal([Constraint::Percentage(65), Constraint::Percentage(35)])
                .areas(bottom);
        draw_log(
            frame,
            log_area,
            "Output",
            &self.log,
            self.log_scroll,
            self.focus == PANE_LOG,
        );
        draw_log(
            frame,
            dmesg_area,
            "Kernel log",
            &self.dmesg,
            self.dmesg_scroll,
            self.focus == PANE_DMESG,
        );
        hits.add(log_area, Hit::Pane(PANE_LOG));
        hits.add(dmesg_area, Hit::Pane(PANE_DMESG));

        draw_key_hints(frame, hints, &self.key_hints(), hits);
    }
}

/// Tail view of a line buffer. Long lines wrap to the pane width, and
/// `scroll` counts wrapped (screen) lines up from the end.
pub fn draw_log(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    lines: &VecDeque<String>,
    scroll: usize,
    focused: bool,
) {
    let title = if scroll > 0 {
        format!("{title} (scrolled {scroll})")
    } else {
        title.to_string()
    };
    let block = pane_block(&title, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let width = inner.width.max(1) as usize;
    let visible = inner.height as usize;
    // Only wrap as many logical lines as could possibly fill the pane plus the scroll.
    let needed = visible + scroll;
    let mut wrapped: Vec<Line> = Vec::new();
    for text in lines.iter().rev() {
        let pieces = wrap_line(text, width);
        for piece in pieces.into_iter().rev() {
            wrapped.push(style_log_line(&piece, text));
        }
        if wrapped.len() >= needed {
            break;
        }
    }
    wrapped.reverse();
    let end = wrapped.len().saturating_sub(scroll);
    let start = end.saturating_sub(visible);
    frame.render_widget(Paragraph::new(wrapped[start..end].to_vec()), inner);
}

/// Split a line into pieces no wider than `width` characters.
fn wrap_line(text: &str, width: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return vec![String::new()];
    }
    chars.chunks(width).map(|c| c.iter().collect()).collect()
}

/// Style a (possibly wrapped) piece using the whole original line for context.
fn style_log_line(piece: &str, whole: &str) -> Line<'static> {
    let text = whole;
    let lower = text.to_ascii_lowercase();
    let style = if lower.contains("fail")
        || lower.contains("error")
        || lower.contains("mismatch")
        || lower.contains("diff ")
    {
        Style::default().fg(theme::BAD)
    } else if text.starts_with("===") || text.starts_with("kiln:") {
        Style::default().fg(theme::INFO).bold()
    } else if text.starts_with("KILN_METRIC") {
        theme::dim()
    } else {
        Style::default()
    };
    Line::from(Span::styled(piece.to_string(), style))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lines_wrap_and_empty_lines_survive() {
        assert_eq!(wrap_line("abcdefgh", 3), vec!["abc", "def", "gh"]);
        assert_eq!(wrap_line("", 3), vec![""]);
        assert_eq!(wrap_line("ab", 10), vec!["ab"]);
    }
}
