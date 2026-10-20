//! Pick which tests to run. Left: checklist. Right: details of the highlighted test.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::AppState;
use crate::engine::plan::Profile;
use crate::engine::util::format_duration_secs;
use crate::engine::{DepReport, Plan};

use super::super::input::{Action, Hit, HitMap};
use super::super::theme;
use super::super::widgets::{clamp_scroll, draw_key_hints, pane_block};
use super::{Effect, Screen};

const PANE_LIST: usize = 0;
const PANE_DETAIL: usize = 1;
const PANE_PLANS: usize = 2;

pub struct PickerScreen {
    pub selected: Vec<bool>,
    cursor: usize,
    offset: usize,
    detail_scroll: u16,
    focus: usize,
    deps: Vec<DepReport>,
    /// Built-in plans followed by saved ones.
    plans: Vec<Plan>,
    plan_cursor: usize,
}

impl PickerScreen {
    pub fn new(state: &AppState) -> Self {
        let n = state.registry.len();
        let mut screen = Self {
            selected: vec![false; n],
            cursor: 0,
            offset: 0,
            detail_scroll: 0,
            focus: PANE_PLANS,
            deps: Vec::new(),
            plans: Vec::new(),
            plan_cursor: 0,
        };
        screen.refresh(state);
        screen
    }

    /// Re-check dependencies (tools may have been installed meanwhile) and
    /// reload plans (one may have just been saved).
    pub fn refresh(&mut self, state: &AppState) {
        self.deps = state
            .registry
            .iter()
            .map(|t| DepReport::check(&t.deps()))
            .collect();
        self.plans = Plan::all(&state.paths.plans_dir);
        self.plan_cursor = self.plan_cursor.min(self.plans.len().saturating_sub(1));
    }

    /// Tab order: plans, tests, details.
    fn next_pane(&self, backwards: bool) -> usize {
        let order = [PANE_PLANS, PANE_LIST, PANE_DETAIL];
        let at = order.iter().position(|p| *p == self.focus).unwrap_or(0);
        let next = if backwards {
            (at + order.len() - 1) % order.len()
        } else {
            (at + 1) % order.len()
        };
        order[next]
    }

    pub fn key_hints(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("Space", "toggle"),
            ("a", "all"),
            ("n", "none"),
            ("Enter", "set up plan / run"),
            ("Tab", "plans / tests"),
            ("t", "history"),
            ("o", "settings"),
            ("?", "help"),
            ("q", "quit"),
        ]
    }

    pub fn handle(&mut self, action: Action, state: &AppState, hits: &HitMap) -> Vec<Effect> {
        let n = self.selected.len();
        match action {
            Action::Up => match self.focus {
                PANE_LIST => self.cursor = self.cursor.saturating_sub(1),
                PANE_PLANS => self.plan_cursor = self.plan_cursor.saturating_sub(1),
                _ => self.detail_scroll = self.detail_scroll.saturating_sub(1),
            },
            Action::Down => match self.focus {
                PANE_LIST => self.cursor = (self.cursor + 1).min(n.saturating_sub(1)),
                PANE_PLANS => {
                    self.plan_cursor =
                        (self.plan_cursor + 1).min(self.plans.len().saturating_sub(1))
                }
                _ => self.detail_scroll += 1,
            },
            Action::Home => match self.focus {
                PANE_PLANS => self.plan_cursor = 0,
                _ => self.cursor = 0,
            },
            Action::End => match self.focus {
                PANE_PLANS => self.plan_cursor = self.plans.len().saturating_sub(1),
                _ => self.cursor = n.saturating_sub(1),
            },
            Action::PageUp => self.detail_scroll = self.detail_scroll.saturating_sub(10),
            Action::PageDown => self.detail_scroll += 10,
            Action::Left => {
                self.focus = if self.focus == PANE_DETAIL {
                    PANE_LIST
                } else {
                    PANE_PLANS
                }
            }
            Action::Right => {
                self.focus = if self.focus == PANE_PLANS {
                    PANE_LIST
                } else {
                    PANE_DETAIL
                }
            }
            Action::NextPane => self.focus = self.next_pane(false),
            Action::PrevPane => self.focus = self.next_pane(true),
            // Space only toggles in the test list; on a plan it opens it like Enter.
            Action::Toggle if self.focus == PANE_PLANS => {
                return match self.plans.get(self.plan_cursor) {
                    Some(plan) => vec![Effect::SetupPlan(Box::new(plan.clone()))],
                    None => Vec::new(),
                };
            }
            Action::Toggle if self.focus == PANE_LIST => self.toggle(self.cursor),
            Action::Char('a') => {
                for (i, s) in self.selected.iter_mut().enumerate() {
                    *s = self.deps[i].all_ok();
                }
            }
            Action::Char('n') => self.selected.iter_mut().for_each(|s| *s = false),
            Action::Confirm | Action::Char('r') if self.focus == PANE_PLANS => {
                return match self.plans.get(self.plan_cursor) {
                    Some(plan) => vec![Effect::SetupPlan(Box::new(plan.clone()))],
                    None => Vec::new(),
                };
            }
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
                Some(Hit::Row { pane, index }) if *pane == PANE_PLANS => {
                    self.focus = PANE_PLANS;
                    if self.plan_cursor == *index
                        && let Some(plan) = self.plans.get(*index)
                    {
                        return vec![Effect::SetupPlan(Box::new(plan.clone()))];
                    }
                    self.plan_cursor = *index;
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

        let plans_height = (self.plans.len() as u16 + 2).clamp(3, 10);
        let [plans_area, left] =
            Layout::vertical([Constraint::Length(plans_height), Constraint::Min(4)]).areas(left);

        // Plans.
        let block = pane_block("Plans", self.focus == PANE_PLANS);
        let inner = block.inner(plans_area);
        frame.render_widget(block, plans_area);
        hits.add(plans_area, Hit::Pane(PANE_PLANS));
        let visible = inner.height as usize;
        let plan_offset = clamp_scroll(self.plan_cursor, self.plans.len(), visible, 0);
        let mut lines = Vec::new();
        for (i, plan) in self
            .plans
            .iter()
            .enumerate()
            .skip(plan_offset)
            .take(visible)
        {
            let style = if i == self.plan_cursor && self.focus == PANE_PLANS {
                theme::selected()
            } else {
                Style::default()
            };
            let kind = if plan.builtin { "" } else { "saved" };
            let destructive = plan.tests.iter().any(|t| {
                state
                    .registry
                    .get(&t.test)
                    .is_some_and(|x| x.info().destructive)
            });
            let mut spans = vec![
                Span::raw(format!(" {:<24}", plan.name)),
                Span::styled(format!("{:<6}", kind), theme::dim()),
            ];
            if destructive {
                spans.push(Span::styled("wipes", Style::default().fg(theme::BAD)));
            }
            lines.push(Line::from(spans).style(style));
            hits.add(
                Rect::new(inner.x, inner.y + (i - plan_offset) as u16, inner.width, 1),
                Hit::Row {
                    pane: PANE_PLANS,
                    index: i,
                },
            );
        }
        frame.render_widget(Paragraph::new(lines), inner);

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
            // Filled bar where the cursor is; a quiet marker on the item the
            // Details pane is describing when focus is over there.
            let mut style = if i == self.cursor && self.focus == PANE_LIST {
                theme::selected()
            } else if i == self.cursor && self.focus == PANE_DETAIL {
                theme::selected_inactive()
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
        if self.focus == PANE_PLANS {
            if let Some(plan) = self.plans.get(self.plan_cursor) {
                let lines = plan_details(plan, state);
                frame.render_widget(
                    Paragraph::new(lines)
                        .wrap(Wrap { trim: false })
                        .scroll((self.detail_scroll, 0)),
                    inner,
                );
            }
        } else if let Some(test) = state.registry.iter().nth(self.cursor) {
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

/// The right pane when a plan is highlighted: what it runs and how long.
fn plan_details<'a>(plan: &'a Plan, state: &AppState) -> Vec<Line<'a>> {
    let mut lines = vec![
        Line::from(Span::styled(plan.name.clone(), theme::title())),
        Line::from(Span::styled(
            if plan.builtin {
                "built-in plan"
            } else {
                "saved plan"
            },
            theme::dim(),
        )),
        Line::from(""),
        Line::from(plan.description.clone()),
        Line::from(""),
        Line::from(Span::styled(
            format!(
                "Tests  (about {} of fixed-length work)",
                format_duration_secs(plan.estimated_secs())
            ),
            theme::title(),
        )),
    ];
    for (i, entry) in plan.tests.iter().enumerate() {
        let name = state
            .registry
            .get(&entry.test)
            .map(|t| t.info().name)
            .unwrap_or("unknown test");
        let params: Vec<String> = entry
            .params
            .iter()
            .map(|(k, v)| format!("{k}={}", v.to_display()))
            .collect();
        let repeat = if entry.repeat > 1 {
            format!("  x{}", entry.repeat)
        } else {
            String::new()
        };
        lines.push(Line::from(vec![
            Span::raw(format!("  {:<2} {name}{repeat}", i + 1)),
            Span::styled(
                if params.is_empty() {
                    String::new()
                } else {
                    format!("   {}", params.join(" "))
                },
                theme::dim(),
            ),
        ]));
    }
    let g = &plan.guards;
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Guards", theme::title())));
    lines.push(Line::from(format!(
        "  temperature cap {}   time cap {}   cooldown {}s   stop on fail {}",
        g.temp_max_c
            .map(|c| format!("{c:.0}°C"))
            .unwrap_or_else(|| "off".into()),
        g.time_max_secs
            .map(format_duration_secs)
            .unwrap_or_else(|| "off".into()),
        g.cooldown_secs,
        if g.stop_on_fail { "yes" } else { "no" }
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Enter opens it in Setup, where every value can still be changed before starting.",
        theme::help(),
    )));
    lines
}
