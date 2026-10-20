//! Past runs, newest first. Lets you reopen one, write a report or delete it.

use std::path::PathBuf;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::AppState;
use crate::engine::Run;
use crate::engine::util::format_duration_secs;

use super::super::input::{Action, Hit, HitMap};
use super::super::theme;
use super::super::widgets::{Modal, clamp_scroll, draw_key_hints, pane_block};
use super::{Effect, Screen};

pub struct HistoryScreen {
    runs: Vec<(PathBuf, Run)>,
    cursor: usize,
    offset: usize,
}

impl HistoryScreen {
    pub fn new() -> Self {
        Self {
            runs: Vec::new(),
            cursor: 0,
            offset: 0,
        }
    }

    pub fn refresh(&mut self, state: &AppState) {
        self.runs = Run::list(&state.paths.runs_dir);
        self.cursor = self.cursor.min(self.runs.len().saturating_sub(1));
    }

    pub fn key_hints(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("Enter", "open"),
            ("x", "delete"),
            ("Esc", "back"),
            ("?", "help"),
        ]
    }

    pub fn handle(&mut self, action: Action, hits: &HitMap) -> Vec<Effect> {
        let len = self.runs.len();
        match action {
            Action::Up => self.cursor = self.cursor.saturating_sub(1),
            Action::Down => self.cursor = (self.cursor + 1).min(len.saturating_sub(1)),
            Action::Home => self.cursor = 0,
            Action::End => self.cursor = len.saturating_sub(1),
            Action::Confirm => return self.open(),
            Action::Char('x') => {
                if let Some((_, run)) = self.runs.get(self.cursor) {
                    return vec![Effect::Modal(Modal::confirm(
                        "Delete run",
                        &format!("Delete run {} and its logs?", run.id),
                        "delete-run",
                    ))];
                }
            }
            Action::Back => return vec![Effect::Goto(Screen::Picker)],
            Action::Click { x, y } => {
                if let Some(Hit::Row { index, .. }) = hits.find(x, y) {
                    if self.cursor == *index {
                        return self.open();
                    }
                    self.cursor = *index;
                }
            }
            Action::Scroll { up, .. } => {
                self.cursor = if up {
                    self.cursor.saturating_sub(1)
                } else {
                    (self.cursor + 1).min(len.saturating_sub(1))
                }
            }
            _ => {}
        }
        Vec::new()
    }

    fn open(&self) -> Vec<Effect> {
        match self.runs.get(self.cursor) {
            Some((dir, run)) => vec![Effect::ShowRun {
                run: Box::new(run.clone()),
                dir: dir.clone(),
                from_history: true,
            }],
            None => Vec::new(),
        }
    }

    /// Delete the highlighted run's directory. Called after the modal confirms.
    pub fn delete_current(&mut self, state: &AppState) -> Vec<Effect> {
        let Some((dir, run)) = self.runs.get(self.cursor) else {
            return Vec::new();
        };
        let id = run.id.clone();
        let effect = match std::fs::remove_dir_all(dir) {
            Ok(()) => Effect::Toast(format!("deleted run {id}")),
            Err(e) => Effect::Toast(format!("could not delete: {e}")),
        };
        self.refresh(state);
        vec![effect]
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, state: &AppState, hits: &mut HitMap) {
        let [body, hints] =
            Layout::vertical([Constraint::Min(5), Constraint::Length(1)]).areas(area);
        let block = pane_block(
            &format!("History  ({})", state.paths.runs_dir.display()),
            true,
        );
        let inner = block.inner(body);
        frame.render_widget(block, body);
        hits.add(body, Hit::Pane(0));

        if self.runs.is_empty() {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled("  no runs yet", theme::dim()))),
                inner,
            );
        }
        let visible = inner.height as usize;
        self.offset = clamp_scroll(self.cursor, self.runs.len(), visible, self.offset);
        let mut lines = Vec::new();
        for (i, (_, run)) in self.runs.iter().enumerate().skip(self.offset).take(visible) {
            let state_label = if run.in_progress {
                "UNFINISHED"
            } else if run.all_passed() {
                "PASS"
            } else {
                "FAIL"
            };
            let style = match state_label {
                "PASS" => Style::default().fg(theme::OK),
                "FAIL" => Style::default().fg(theme::BAD),
                _ => Style::default().fg(theme::WARN),
            };
            let secs = (run.ended - run.started).num_seconds().max(0) as u64;
            let tests: Vec<String> = run
                .results
                .iter()
                .map(|r| format!("{}:{}", r.test_id, r.status.label()))
                .collect();
            let mut line = Line::from(vec![
                Span::raw(format!(" {}  ", run.started.format("%Y-%m-%d %H:%M"))),
                Span::styled(format!("{state_label:<10}"), style),
                Span::styled(format!("{:>8}  ", format_duration_secs(secs)), theme::dim()),
                Span::raw(tests.join(" ")),
            ]);
            if i == self.cursor {
                line = line.style(theme::selected());
            }
            lines.push(line);
            hits.add(
                Rect::new(inner.x, inner.y + (i - self.offset) as u16, inner.width, 1),
                Hit::Row { pane: 0, index: i },
            );
        }
        frame.render_widget(Paragraph::new(lines), inner);
        draw_key_hints(frame, hints, &self.key_hints(), hits);
    }
}
