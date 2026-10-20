//! A finished or past run: summary table on the left, details or a log on the right.

use std::collections::VecDeque;
use std::path::PathBuf;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::engine::Run;
use crate::engine::util::format_duration_secs;
use crate::report;

use super::super::input::{Action, Hit, HitMap};
use super::super::theme;
use super::super::widgets::{clamp_scroll, draw_key_hints, pane_block};
use super::dashboard::draw_log;
use super::{Effect, Screen};

const PANE_LIST: usize = 0;
const PANE_DETAIL: usize = 1;

pub struct ResultsScreen {
    run: Option<Run>,
    dir: PathBuf,
    from_history: bool,
    cursor: usize,
    offset: usize,
    focus: usize,
    detail_scroll: u16,
    /// When set, the right pane shows this log instead of the details.
    log: Option<VecDeque<String>>,
    log_scroll: usize,
}

impl ResultsScreen {
    pub fn empty() -> Self {
        Self {
            run: None,
            dir: PathBuf::new(),
            from_history: false,
            cursor: 0,
            offset: 0,
            focus: PANE_LIST,
            detail_scroll: 0,
            log: None,
            log_scroll: 0,
        }
    }

    pub fn show(&mut self, run: Run, dir: PathBuf, from_history: bool) {
        self.run = Some(run);
        self.dir = dir;
        self.from_history = from_history;
        self.cursor = 0;
        self.offset = 0;
        self.focus = PANE_LIST;
        self.detail_scroll = 0;
        self.log = None;
        self.log_scroll = 0;
    }

    pub fn key_hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.log.is_some() {
            vec![("Esc", "close log"), ("Tab", "switch pane")]
        } else {
            vec![
                ("v", "view log"),
                ("m", "write report.md"),
                ("Tab", "switch pane"),
                ("Esc", "back"),
                ("?", "help"),
            ]
        }
    }

    pub fn handle(&mut self, action: Action, hits: &HitMap) -> Vec<Effect> {
        let len = self.run.as_ref().map(|r| r.results.len()).unwrap_or(0);
        match action {
            Action::Up => match (self.focus, &self.log) {
                (PANE_LIST, _) => {
                    self.cursor = self.cursor.saturating_sub(1);
                    self.detail_scroll = 0;
                }
                (_, Some(log)) => {
                    self.log_scroll = (self.log_scroll + 1).min(log.len().saturating_sub(1))
                }
                _ => self.detail_scroll = self.detail_scroll.saturating_sub(1),
            },
            Action::Down => match (self.focus, &self.log) {
                (PANE_LIST, _) => {
                    self.cursor = (self.cursor + 1).min(len.saturating_sub(1));
                    self.detail_scroll = 0;
                }
                (_, Some(_)) => self.log_scroll = self.log_scroll.saturating_sub(1),
                _ => self.detail_scroll += 1,
            },
            Action::PageUp => match &self.log {
                Some(log) => {
                    self.log_scroll = (self.log_scroll + 10).min(log.len().saturating_sub(1))
                }
                None => self.detail_scroll = self.detail_scroll.saturating_sub(10),
            },
            Action::PageDown => match &self.log {
                Some(_) => self.log_scroll = self.log_scroll.saturating_sub(10),
                None => self.detail_scroll += 10,
            },
            Action::End => self.log_scroll = 0,
            Action::NextPane | Action::PrevPane | Action::Left | Action::Right => {
                self.focus = 1 - self.focus
            }
            Action::Confirm | Action::Char('v') => {
                if self.log.is_some() {
                    self.log = None;
                } else {
                    self.open_log();
                }
            }
            Action::Char('m') => {
                if let Some(run) = &self.run {
                    let path = self.dir.join("report.md");
                    return match std::fs::write(&path, report::markdown(run)) {
                        Ok(()) => vec![Effect::Toast(format!("wrote {}", path.display()))],
                        Err(e) => vec![Effect::Toast(format!("could not write report: {e}"))],
                    };
                }
            }
            Action::Back => {
                if self.log.is_some() {
                    self.log = None;
                } else {
                    return vec![Effect::Goto(if self.from_history {
                        Screen::History
                    } else {
                        Screen::Picker
                    })];
                }
            }
            Action::Click { x, y } => match hits.find(x, y) {
                Some(Hit::Row { pane, index }) if *pane == PANE_LIST => {
                    self.focus = PANE_LIST;
                    if self.cursor == *index && self.log.is_none() {
                        self.open_log();
                    }
                    self.cursor = *index;
                    self.detail_scroll = 0;
                }
                Some(Hit::Pane(p)) => self.focus = *p,
                _ => {}
            },
            Action::Scroll { x, y, up } => match (hits.pane_at(x, y), &self.log) {
                (Some(PANE_DETAIL), Some(log)) => {
                    self.log_scroll = if up {
                        (self.log_scroll + 3).min(log.len().saturating_sub(1))
                    } else {
                        self.log_scroll.saturating_sub(3)
                    }
                }
                (Some(PANE_DETAIL), None) => {
                    self.detail_scroll = if up {
                        self.detail_scroll.saturating_sub(3)
                    } else {
                        self.detail_scroll + 3
                    }
                }
                _ => {
                    self.cursor = if up {
                        self.cursor.saturating_sub(1)
                    } else {
                        (self.cursor + 1).min(len.saturating_sub(1))
                    }
                }
            },
            _ => {}
        }
        Vec::new()
    }

    fn open_log(&mut self) {
        let Some(run) = &self.run else { return };
        let Some(result) = run.results.get(self.cursor) else {
            return;
        };
        if result.log_file.is_empty() {
            self.log = Some(VecDeque::from(vec![format!(
                "(no log: {})",
                result.status.detail()
            )]));
        } else {
            let path = self.dir.join(&result.log_file);
            let lines = match std::fs::read_to_string(&path) {
                Ok(text) => text.lines().map(str::to_string).collect(),
                Err(e) => VecDeque::from(vec![format!("could not read {}: {e}", path.display())]),
            };
            self.log = Some(lines);
        }
        self.log_scroll = 0;
        self.focus = PANE_DETAIL;
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, hits: &mut HitMap) {
        let [body, hints] =
            Layout::vertical([Constraint::Min(5), Constraint::Length(1)]).areas(area);
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)])
                .areas(body);

        let Some(run) = &self.run else {
            frame.render_widget(Paragraph::new("no run"), body);
            return;
        };
        let overall = if run.in_progress {
            "UNFINISHED"
        } else if run.all_passed() {
            "PASS"
        } else {
            "FAIL"
        };
        let block = pane_block(
            &format!("Run {}  {overall}", run.id),
            self.focus == PANE_LIST,
        );
        let inner = block.inner(left);
        frame.render_widget(block, left);
        hits.add(left, Hit::Pane(PANE_LIST));
        let visible = inner.height as usize;
        self.offset = clamp_scroll(self.cursor, run.results.len(), visible, self.offset);
        let mut lines = Vec::new();
        for (i, r) in run
            .results
            .iter()
            .enumerate()
            .skip(self.offset)
            .take(visible)
        {
            let secs = (r.ended - r.started).num_seconds().max(0) as u64;
            let temp = r
                .max_temp_c
                .map(|t| format!("{t:.0}°C"))
                .unwrap_or_else(|| "-".into());
            let mut line = Line::from(vec![
                Span::raw(format!(" {:<2} ", i + 1)),
                Span::styled(
                    format!("{:<6}", r.status.label()),
                    theme::status(r.status.label()),
                ),
                Span::raw(format!("{:<24}", r.name)),
                Span::styled(
                    format!("{:>8} {:>5}", format_duration_secs(secs), temp),
                    theme::dim(),
                ),
            ]);
            if i == self.cursor {
                line = line.style(theme::selected());
            }
            lines.push(line);
            hits.add(
                Rect::new(inner.x, inner.y + (i - self.offset) as u16, inner.width, 1),
                Hit::Row {
                    pane: PANE_LIST,
                    index: i,
                },
            );
        }
        frame.render_widget(Paragraph::new(lines), inner);

        hits.add(right, Hit::Pane(PANE_DETAIL));
        if let Some(log) = &self.log {
            let title = run
                .results
                .get(self.cursor)
                .map(|r| r.log_file.clone())
                .unwrap_or_default();
            draw_log(
                frame,
                right,
                &title,
                log,
                self.log_scroll,
                self.focus == PANE_DETAIL,
            );
        } else {
            let block = pane_block("Details", self.focus == PANE_DETAIL);
            let inner = block.inner(right);
            frame.render_widget(block, right);
            let mut lines = Vec::new();
            if let Some(r) = run.results.get(self.cursor) {
                lines.push(Line::from(vec![
                    Span::styled(r.name.clone(), theme::title()),
                    Span::raw("  "),
                    Span::styled(r.status.label(), theme::status(r.status.label())),
                ]));
                let detail = r.status.detail();
                if !detail.is_empty() {
                    lines.push(Line::from(Span::styled(
                        detail,
                        Style::default().fg(theme::WARN),
                    )));
                }
                lines.push(Line::from(""));
                if !r.params.is_empty() {
                    lines.push(Line::from(Span::styled("Parameters", theme::title())));
                    for (k, v) in &r.params {
                        lines.push(Line::from(format!("  {k:<16} {}", v.to_display())));
                    }
                    lines.push(Line::from(""));
                }
                if !r.metrics.is_empty() {
                    lines.push(Line::from(Span::styled("Metrics", theme::title())));
                    for (k, v) in &r.metrics {
                        let text = if v.fract() == 0.0 {
                            format!("{}", *v as i64)
                        } else {
                            format!("{v:.2}")
                        };
                        lines.push(Line::from(format!("  {k:<24} {text}")));
                    }
                    lines.push(Line::from(""));
                }
                for snap in &r.snapshots {
                    let changed = snap.before.trim() != snap.after.trim();
                    lines.push(Line::from(vec![
                        Span::styled(format!("Snapshot {}", snap.name), theme::title()),
                        Span::styled(
                            if changed { "  changed" } else { "  unchanged" },
                            if changed {
                                Style::default().fg(theme::WARN)
                            } else {
                                theme::dim()
                            },
                        ),
                    ]));
                    let text = if changed {
                        format!(
                            "before:\n{}\nafter:\n{}",
                            snap.before.trim(),
                            snap.after.trim()
                        )
                    } else {
                        snap.after.trim().to_string()
                    };
                    for l in text.lines().take(40) {
                        lines.push(Line::from(format!("  {l}")));
                    }
                    lines.push(Line::from(""));
                }
            }
            lines.push(Line::from(Span::styled("Machine", theme::title())));
            for l in run.env.summary_lines() {
                lines.push(Line::from(format!("  {l}")));
            }
            frame.render_widget(
                Paragraph::new(lines)
                    .wrap(Wrap { trim: false })
                    .scroll((self.detail_scroll, 0)),
                inner,
            );
        }

        draw_key_hints(frame, hints, &self.key_hints(), hits);
    }
}
