//! Pick which tests to run. Left: checklist. Right: details of the highlighted test.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::AppState;
use crate::engine::DepReport;
use crate::engine::plan::Profile;

use super::super::input::{Action, Hit, HitMap};
use super::super::theme;
use super::super::widgets::{clamp_scroll, draw_key_hints, pane_block};
use super::{Effect, Screen};

const PANE_LIST: usize = 0;
const PANE_DETAIL: usize = 1;

pub struct PickerScreen {
    pub selected: Vec<bool>,
    cursor: usize,
    offset: usize,
    detail_scroll: u16,
    focus: usize,
    deps: Vec<DepReport>,
}

impl PickerScreen {
    pub fn new(state: &AppState) -> Self {
        let n = state.registry.len();
        let mut screen = Self {
            selected: vec![false; n],
            cursor: 0,
            offset: 0,
            detail_scroll: 0,
            focus: PANE_LIST,
            deps: Vec::new(),
        };
        screen.refresh(state);
        screen
    }

    /// Re-check dependencies (tools may have been installed meanwhile).
    pub fn refresh(&mut self, state: &AppState) {
        self.deps = state
            .registry
            .iter()
            .map(|t| DepReport::check(&t.deps()))
            .collect();
    }

    pub fn key_hints(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("Space", "toggle"),
            ("a", "all"),
            ("n", "none"),
            ("Enter", "set up run"),
            ("t", "history"),
            ("o", "settings"),
            ("?", "help"),
            ("q", "quit"),
        ]
    }

    pub fn handle(&mut self, action: Action, state: &AppState, hits: &HitMap) -> Vec<Effect> {
        let n = self.selected.len();
        match action {
            Action::Up => {
                if self.focus == PANE_LIST {
                    self.cursor = self.cursor.saturating_sub(1);
                } else {
                    self.detail_scroll = self.detail_scroll.saturating_sub(1);
                }
            }
            Action::Down => {
                if self.focus == PANE_LIST {
                    self.cursor = (self.cursor + 1).min(n.saturating_sub(1));
                } else {
                    self.detail_scroll += 1;
                }
            }
            Action::Home => self.cursor = 0,
            Action::End => self.cursor = n.saturating_sub(1),
            Action::PageUp => self.detail_scroll = self.detail_scroll.saturating_sub(10),
            Action::PageDown => self.detail_scroll += 10,
            Action::Left => self.focus = PANE_LIST,
            Action::Right => self.focus = PANE_DETAIL,
            Action::NextPane | Action::PrevPane => self.focus = 1 - self.focus,
            Action::Toggle => self.toggle(self.cursor),
            Action::Char('a') => {
                for (i, s) in self.selected.iter_mut().enumerate() {
                    *s = self.deps[i].all_ok();
                }
            }
            Action::Char('n') => self.selected.iter_mut().for_each(|s| *s = false),
            Action::Confirm | Action::Char('r') => {
                let ids: Vec<String> = state
                    .registry
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| self.selected[*i])
                    .map(|(_, t)| t.info().id.to_string())
                    .collect();
                if ids.is_empty() {
                    return vec![Effect::Toast("select at least one test (Space)".into())];
                }
                return vec![Effect::SetupFor(ids)];
            }
            Action::Char('t') => return vec![Effect::Goto(Screen::History)],
            Action::Char('o') => return vec![Effect::Goto(Screen::Settings)],
            Action::Click { x, y } => match hits.find(x, y) {
                Some(Hit::Row { pane, index }) if *pane == PANE_LIST => {
                    self.focus = PANE_LIST;
                    if self.cursor == *index {
                        self.toggle(*index);
                    }
                    self.cursor = *index;
                    self.detail_scroll = 0;
                }
                Some(Hit::Pane(p)) => self.focus = *p,
                _ => {}
            },
            Action::Scroll { x, y, up } => match hits.pane_at(x, y) {
                Some(PANE_DETAIL) => {
                    self.detail_scroll = if up {
                        self.detail_scroll.saturating_sub(2)
                    } else {
                        self.detail_scroll + 2
                    }
                }
                _ => {
                    self.cursor = if up {
                        self.cursor.saturating_sub(1)
                    } else {
                        (self.cursor + 1).min(n.saturating_sub(1))
                    }
                }
            },
            _ => {}
        }
        if matches!(
            action,
            Action::Up | Action::Down | Action::Home | Action::End
        ) && self.focus == PANE_LIST
        {
            self.detail_scroll = 0;
        }
        Vec::new()
    }

    fn toggle(&mut self, index: usize) {
        if index < self.selected.len() && self.deps[index].all_ok() {
            self.selected[index] = !self.selected[index];
        }
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, state: &AppState, hits: &mut HitMap) {
        let [body, hints] =
            Layout::vertical([Constraint::Min(5), Constraint::Length(1)]).areas(area);
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)])
                .areas(body);

        // Checklist.
        let block = pane_block("Tests", self.focus == PANE_LIST);
        let inner = block.inner(left);
        frame.render_widget(block, left);
        hits.add(left, Hit::Pane(PANE_LIST));
        let visible = inner.height as usize;
        self.offset = clamp_scroll(self.cursor, self.selected.len(), visible, self.offset);
        let mut lines = Vec::new();
        for (i, test) in state
            .registry
            .iter()
            .enumerate()
            .skip(self.offset)
            .take(visible)
        {
            let info = test.info();
            let ok = self.deps[i].all_ok();
            let mark = if self.selected[i] { "[x]" } else { "[ ]" };
            let mut style = if i == self.cursor {
                theme::selected()
            } else {
                Style::default()
            };
            if !ok {
                style = style.fg(theme::DIM);
            }
            let mut spans = vec![
                Span::raw(format!(" {mark} ")),
                Span::raw(format!("{:<26}", info.name)),
                Span::styled(format!("{:<8}", info.category.label()), theme::dim()),
            ];
            if info.destructive {
                spans.push(Span::styled("wipes", Style::default().fg(theme::BAD)));
            } else if !ok {
                spans.push(Span::styled("missing", Style::default().fg(theme::WARN)));
            }
            lines.push(Line::from(spans).style(style));
            let row = Rect::new(inner.x, inner.y + (i - self.offset) as u16, inner.width, 1);
            hits.add(
                row,
                Hit::Row {
                    pane: PANE_LIST,
                    index: i,
                },
            );
        }
        frame.render_widget(Paragraph::new(lines), inner);

        // Details.
        let block = pane_block("Details", self.focus == PANE_DETAIL);
        let inner = block.inner(right);
        frame.render_widget(block, right);
        hits.add(right, Hit::Pane(PANE_DETAIL));
        if let Some(test) = state.registry.iter().nth(self.cursor) {
            let info = test.info();
            let mut lines = vec![
                Line::from(Span::styled(info.name, theme::title())),
                Line::from(Span::styled(
                    format!("{}  ·  {}", info.id, info.category.label()),
                    theme::dim(),
                )),
                Line::from(""),
                Line::from(Span::raw(info.tldr).bold()),
                Line::from(""),
            ];
            for para in info.description.split("\n\n") {
                lines.push(Line::from(para.replace('\n', " ")));
                lines.push(Line::from(""));
            }
            lines.push(Line::from(Span::styled("Uses", theme::title())));
            for u in info.uses {
                lines.push(Line::from(format!("  {u}")));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("Dependencies", theme::title())));
            let report = &self.deps[self.cursor];
            if report.statuses.is_empty() {
                lines.push(Line::from("  none"));
            }
            for s in &report.statuses {
                let (mark, style) = if s.ok {
                    ("ok", Style::default().fg(theme::OK))
                } else {
                    ("missing", Style::default().fg(theme::WARN))
                };
                let hint = s
                    .hint
                    .as_ref()
                    .map(|h| format!("   {h}"))
                    .unwrap_or_default();
                lines.push(Line::from(vec![
                    Span::styled(format!("  {mark:<8}"), style),
                    Span::raw(s.dep.label()),
                    Span::styled(hint, theme::dim()),
                ]));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("Parameters", theme::title())));
            let params = test.params();
            if params.is_empty() {
                lines.push(Line::from("  none"));
            }
            for p in &params {
                let default = p
                    .default_value()
                    .map(|v| v.to_display())
                    .unwrap_or_else(|| "(required)".into());
                lines.push(Line::from(vec![
                    Span::raw(format!("  {:<16}", p.label)),
                    Span::styled(format!("{default:<12}"), Style::default().fg(theme::INFO)),
                    Span::styled(p.help.to_string(), theme::dim()),
                ]));
            }
            let profiles = Profile::list(&state.paths.profiles_for(info.id));
            if !profiles.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled("Saved profiles", theme::title())));
                for p in profiles {
                    lines.push(Line::from(format!("  {}  {}", p.name, p.description)));
                }
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
