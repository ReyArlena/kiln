//! Configure guards and parameters for the chosen tests, then start the run.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::AppState;
use crate::engine::params::{ParamKind, ParamSpec, ParamValue};
use crate::engine::plan::Profile;
use crate::engine::util::{format_duration_secs, parse_duration_secs};
use crate::engine::{Guards, ParamSet, Plan, PlanEntry};

use super::super::input::{Action, Hit, HitMap};
use super::super::theme;
use super::super::widgets::{Modal, TextInput, clamp_scroll, draw_key_hints, pane_block};
use super::Effect;

/// The form rows. Headings are not selectable.
enum Row {
    Heading(String),
    Guard(GuardField),
    Profile {
        test: usize,
    },
    Param {
        test: usize,
        spec: ParamSpec,
    },
    /// How many times to run this test back to back.
    Repeat {
        test: usize,
    },
    Start,
}

#[derive(Clone, Copy, PartialEq)]
enum GuardField {
    TempMax,
    TimeMax,
    Cooldown,
    StopOnFail,
}

struct TestEntry {
    id: String,
    name: String,
    destructive: bool,
    profiles: Vec<Profile>,
    /// 0 = built-in defaults, otherwise `profiles[idx - 1]`.
    profile_idx: usize,
    /// Current text for each parameter, keyed by spec key.
    values: ParamSet,
    repeat: u32,
}

pub struct SetupScreen {
    /// Name of the plan this setup came from, used when saving.
    plan_name: String,
    rows: Vec<Row>,
    tests: Vec<TestEntry>,
    guards: Guards,
    cursor: usize,
    offset: usize,
    editing: Option<TextInput>,
    errors: Vec<Option<String>>,
}

impl SetupScreen {
    pub fn empty() -> Self {
        Self {
            plan_name: String::new(),
            rows: Vec::new(),
            tests: Vec::new(),
            guards: Guards::default(),
            cursor: 0,
            offset: 0,
            editing: None,
            errors: Vec::new(),
        }
    }

    /// Build the form for the given test ids using the app's default guards.
    pub fn new(state: &AppState, ids: &[String]) -> Self {
        let mut tests = Vec::new();
        for id in ids {
            let Some(test) = state.registry.get(id) else {
                continue;
            };
            let info = test.info();
            let mut values = ParamSet::new();
            for spec in test.params() {
                if let Some(v) = spec.default_value() {
                    values.insert(spec.key.to_string(), v);
                }
            }
            tests.push(TestEntry {
                id: id.clone(),
                name: info.name.to_string(),
                destructive: info.destructive,
                profiles: Profile::list(&state.paths.profiles_for(id)),
                profile_idx: 0,
                values,
                repeat: 1,
            });
        }
        let mut screen = Self {
            plan_name: String::new(),
            rows: Vec::new(),
            tests,
            guards: state.config.guards.clone(),
            cursor: 0,
            offset: 0,
            editing: None,
            errors: Vec::new(),
        };
        screen.rebuild_rows(state);
        screen.cursor = screen
            .rows
            .iter()
            .position(|r| !matches!(r, Row::Heading(_)))
            .unwrap_or(0);
        screen
    }

    /// Build the form from a saved or built-in plan: its tests, parameters,
    /// repeats and guards, ready to tweak and start.
    pub fn from_plan(state: &AppState, plan: &Plan) -> Self {
        let ids: Vec<String> = plan.tests.iter().map(|e| e.test.clone()).collect();
        let mut screen = Self::new(state, &ids);
        screen.plan_name = plan.name.clone();
        screen.guards = plan.guards.clone();
        // `new` keeps only the ids it knows, so walk both lists together by id.
        let mut entries = plan.tests.iter();
        for test in &mut screen.tests {
            let Some(entry) = entries.by_ref().find(|e| e.test == test.id) else {
                continue;
            };
            test.repeat = entry.repeat.max(1);
            if let Some(def) = state.registry.get(&test.id) {
                let specs = def.params();
                for (key, value) in &entry.params {
                    // Normalise through the spec so loaded values keep their variant.
                    if let Some(spec) = specs.iter().find(|s| s.key == key)
                        && let Ok(v) = spec.parse(&value.to_text())
                    {
                        test.values.insert(key.clone(), v);
                    }
                }
            }
        }
        screen.rebuild_rows(state);
        screen.cursor = screen
            .rows
            .iter()
            .position(|r| !matches!(r, Row::Heading(_)))
            .unwrap_or(0);
        screen
    }

    fn rebuild_rows(&mut self, state: &AppState) {
        let mut rows = vec![
            Row::Heading("Guards (apply to every test)".into()),
            Row::Guard(GuardField::TempMax),
            Row::Guard(GuardField::TimeMax),
            Row::Guard(GuardField::Cooldown),
            Row::Guard(GuardField::StopOnFail),
        ];
        for (i, entry) in self.tests.iter().enumerate() {
            rows.push(Row::Heading(format!(
                "{}. {}{}",
                i + 1,
                entry.name,
                if entry.destructive {
                    "   (wipes the device)"
                } else {
                    ""
                }
            )));
            if let Some(test) = state.registry.get(&entry.id) {
                let specs = test.params();
                if !entry.profiles.is_empty() {
                    rows.push(Row::Profile { test: i });
                }
                if specs.is_empty() {
                    rows.push(Row::Heading("   no parameters".into()));
                }
                for spec in specs {
                    rows.push(Row::Param { test: i, spec });
                }
                rows.push(Row::Repeat { test: i });
            }
        }
        rows.push(Row::Heading(String::new()));
        rows.push(Row::Start);
        self.errors = vec![None; rows.len()];
        self.rows = rows;
    }

    pub fn is_editing(&self) -> bool {
        self.editing.is_some()
    }

    pub fn key_hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.editing.is_some() {
            vec![("Enter", "apply"), ("Esc", "cancel edit")]
        } else {
            vec![
                ("Enter", "edit / start"),
                ("Space", "toggle / cycle"),
                ("r", "start run"),
                ("w", "save as plan"),
                ("Esc", "back"),
                ("?", "help"),
            ]
        }
    }

    fn value_text(&self, row: &Row) -> String {
        match row {
            Row::Guard(GuardField::TempMax) => self
                .guards
                .temp_max_c
                .map(|c| format!("{c:.0}"))
                .unwrap_or_else(|| "off".into()),
            Row::Guard(GuardField::TimeMax) => self
                .guards
                .time_max_secs
                .map(format_duration_secs)
                .unwrap_or_else(|| "off".into()),
            Row::Guard(GuardField::Cooldown) => self.guards.cooldown_secs.to_string(),
            Row::Guard(GuardField::StopOnFail) => if self.guards.stop_on_fail {
                "yes"
            } else {
                "no"
            }
            .to_string(),
            Row::Profile { test } => {
                let t = &self.tests[*test];
                if t.profile_idx == 0 {
                    "built-in defaults".to_string()
                } else {
                    t.profiles[t.profile_idx - 1].name.clone()
                }
            }
            Row::Param { test, spec } => self.tests[*test]
                .values
                .get(spec.key)
                .map(|v| v.to_display())
                .unwrap_or_default(),
            Row::Repeat { test } => self.tests[*test].repeat.to_string(),
            Row::Heading(_) | Row::Start => String::new(),
        }
    }

    fn selectable(&self, idx: usize) -> bool {
        !matches!(self.rows.get(idx), Some(Row::Heading(_)) | None)
    }

    fn move_cursor(&mut self, delta: i32) {
        let mut idx = self.cursor as i32;
        loop {
            idx += delta;
            if idx < 0 || idx >= self.rows.len() as i32 {
                return;
            }
            if self.selectable(idx as usize) {
                self.cursor = idx as usize;
                return;
            }
        }
    }

    pub fn handle(&mut self, action: Action, state: &AppState, hits: &HitMap) -> Vec<Effect> {
        if let Some(input) = &mut self.editing {
            if let Action::Key(key) = action {
                input.handle_key(key)
            }
            return Vec::new();
        }
        match action {
            Action::Up => self.move_cursor(-1),
            Action::Down => self.move_cursor(1),
            Action::PageUp => (0..8).for_each(|_| self.move_cursor(-1)),
            Action::PageDown => (0..8).for_each(|_| self.move_cursor(1)),
            Action::Home => {
                self.cursor = 0;
                self.move_cursor(1);
            }
            Action::End => self.cursor = self.rows.len() - 1,
            Action::Toggle => self.toggle_current(state),
            Action::Confirm => {
                if matches!(self.rows[self.cursor], Row::Start) {
                    return self.start(state);
                }
                self.begin_edit();
            }
            Action::Char('r') => return self.start(state),
            Action::Char('w') => {
                return vec![Effect::Modal(Modal::input(
                    "Save as plan",
                    "Name for this set of tests and settings:",
                    &self.plan_name,
                    "save-plan",
                ))];
            }
            Action::Back => return vec![Effect::Goto(super::Screen::Picker)],
            Action::Click { x, y } => {
                if let Some(Hit::Row { index, .. }) = hits.find(x, y)
                    && self.selectable(*index)
                {
                    if self.cursor == *index {
                        // Second click acts like Enter or Space depending on the row.
                        if matches!(self.rows[*index], Row::Start) {
                            return self.start(state);
                        } else if self.is_toggle_row(*index) {
                            self.toggle_current(state);
                        } else {
                            self.begin_edit();
                        }
                    }
                    self.cursor = *index;
                }
            }
            Action::Scroll { up, .. } => self.move_cursor(if up { -1 } else { 1 }),
            _ => {}
        }
        Vec::new()
    }

    fn is_toggle_row(&self, idx: usize) -> bool {
        match &self.rows[idx] {
            Row::Guard(GuardField::StopOnFail) | Row::Profile { .. } => true,
            Row::Param { spec, .. } => {
                matches!(spec.kind, ParamKind::Bool { .. } | ParamKind::Choice { .. })
            }
            _ => false,
        }
    }

    fn begin_edit(&mut self) {
        if self.is_toggle_row(self.cursor) {
            return;
        }
        if matches!(
            self.rows[self.cursor],
            Row::Guard(_) | Row::Param { .. } | Row::Repeat { .. }
        ) {
            let current = self.value_text(&self.rows[self.cursor]);
            let current = if current == "(required)" {
                String::new()
            } else {
                current
            };
            self.editing = Some(TextInput::with_value(&current));
        }
    }

    /// Apply the edit buffer to the current row. Returns an error message if invalid.
    pub fn commit_edit(&mut self) {
        let Some(input) = self.editing.take() else {
            return;
        };
        let text = input.value.trim().to_string();
        let result: Result<(), String> = match &self.rows[self.cursor] {
            Row::Guard(GuardField::TempMax) => {
                if text.eq_ignore_ascii_case("off") || text.is_empty() {
                    self.guards.temp_max_c = None;
                    Ok(())
                } else {
                    text.parse::<f32>()
                        .map(|c| self.guards.temp_max_c = Some(c))
                        .map_err(|_| "number of degrees, or `off`".to_string())
                }
            }
            Row::Guard(GuardField::TimeMax) => {
                if text.eq_ignore_ascii_case("off") || text.is_empty() {
                    self.guards.time_max_secs = None;
                    Ok(())
                } else {
                    parse_duration_secs(&text)
                        .map(|s| self.guards.time_max_secs = Some(s))
                        .ok_or_else(|| "duration like 30m, or `off`".to_string())
                }
            }
            Row::Guard(GuardField::Cooldown) => text
                .parse::<u64>()
                .map(|s| self.guards.cooldown_secs = s)
                .map_err(|_| "whole number of seconds".to_string()),
            Row::Param { test, spec } => match spec.parse(&text) {
                Ok(value) => {
                    self.tests[*test].values.insert(spec.key.to_string(), value);
                    Ok(())
                }
                Err(e) => Err(e.to_string()),
            },
            Row::Repeat { test } => text
                .parse::<u32>()
                .ok()
                .filter(|n| (1..=1000).contains(n))
                .map(|n| self.tests[*test].repeat = n)
                .ok_or_else(|| "between 1 and 1000".to_string()),
            _ => Ok(()),
        };
        self.errors[self.cursor] = result.err();
    }

    pub fn cancel_edit(&mut self) {
        self.editing = None;
    }

    fn toggle_current(&mut self, state: &AppState) {
        match &self.rows[self.cursor] {
            Row::Guard(GuardField::StopOnFail) => {
                self.guards.stop_on_fail = !self.guards.stop_on_fail
            }
            Row::Profile { test } => {
                let t = &mut self.tests[*test];
                t.profile_idx = (t.profile_idx + 1) % (t.profiles.len() + 1);
                // Reset to defaults, then layer the profile on top.
                if let Some(def) = state.registry.get(&t.id) {
                    t.values.clear();
                    for spec in def.params() {
                        if let Some(v) = spec.default_value() {
                            t.values.insert(spec.key.to_string(), v);
                        }
                    }
                    if t.profile_idx > 0 {
                        for (k, v) in &t.profiles[t.profile_idx - 1].params {
                            t.values.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
            Row::Param { test, spec } => {
                let entry = &mut self.tests[*test];
                match &spec.kind {
                    ParamKind::Bool { .. } => {
                        let current = entry
                            .values
                            .get(spec.key)
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        entry
                            .values
                            .insert(spec.key.to_string(), ParamValue::Bool(!current));
                    }
                    ParamKind::Choice { choices, .. } => {
                        let current = entry
                            .values
                            .get(spec.key)
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let idx = choices
                            .iter()
                            .position(|c| *c == current)
                            .map(|i| (i + 1) % choices.len())
                            .unwrap_or(0);
                        entry.values.insert(
                            spec.key.to_string(),
                            ParamValue::Text(choices[idx].to_string()),
                        );
                    }
                    _ => self.begin_edit(),
                }
            }
            Row::Start => {}
            _ => {}
        }
    }

    /// Validate everything and build the plan. Destructive tests get a typed confirmation first.
    fn start(&mut self, state: &AppState) -> Vec<Effect> {
        let mut first_error = None;
        for (i, row) in self.rows.iter().enumerate() {
            if let Row::Param { test, spec } = row {
                let err = match self.tests[*test].values.get(spec.key) {
                    Some(v) => spec.validate(v).err().map(|e| e.to_string()),
                    None => Some("required".into()),
                };
                if err.is_some() && first_error.is_none() {
                    first_error = Some(i);
                }
                self.errors[i] = err;
            }
        }
        if let Some(i) = first_error {
            self.cursor = i;
            return vec![Effect::Toast("fix the highlighted parameter first".into())];
        }
        if !state.is_root() {
            let needs_root = self.tests.iter().any(|t| {
                state
                    .registry
                    .get(&t.id)
                    .map(|test| test.deps().contains(&crate::engine::Dep::Root))
                    .unwrap_or(false)
            });
            if needs_root {
                return vec![Effect::Modal(Modal::message(
                    "Root needed",
                    "One of these tests needs root.\nQuit and relaunch with:  sudo kiln",
                ))];
            }
        }
        let devices: Vec<String> = self
            .tests
            .iter()
            .filter(|t| t.destructive)
            .filter_map(|t| t.values.get("device").map(|v| v.to_text()))
            .collect();
        if !devices.is_empty() && state.config.confirm_destructive {
            let device = devices[0].clone();
            let body = format!(
                "These tests will WIPE:\n  {}\nEverything on the device is lost.",
                devices.join("\n  ")
            );
            return vec![Effect::Modal(Modal::typed(
                "Destructive test",
                &body,
                &device,
                "start",
            ))];
        }
        vec![Effect::StartRun(self.build_plan())]
    }

    /// Called by the app when the destructive confirmation succeeds.
    pub fn confirmed_start(&self) -> Effect {
        Effect::StartRun(self.build_plan())
    }

    fn build_plan(&self) -> Plan {
        let name = if self.plan_name.is_empty() {
            "tui"
        } else {
            &self.plan_name
        };
        let mut plan = Plan::new(name, self.guards.clone());
        for t in &self.tests {
            let mut entry = PlanEntry::new(&t.id);
            entry.params = t.values.clone();
            entry.repeat = t.repeat;
            plan.tests.push(entry);
        }
        plan
    }

    /// Save the current form as a named plan. Called after the name prompt.
    pub fn save_plan(&mut self, state: &AppState, name: &str) -> Vec<Effect> {
        let name = name.trim();
        if name.is_empty() {
            return vec![Effect::Toast("plan not saved: empty name".into())];
        }
        self.plan_name = name.to_string();
        let plan = self.build_plan();
        match plan.save_in(&state.paths.plans_dir) {
            Ok(path) => {
                state.paths.restore_owner(&path);
                vec![Effect::Toast(format!(
                    "saved plan `{name}` to {}",
                    path.display()
                ))]
            }
            Err(e) => vec![Effect::Toast(format!("could not save plan: {e}"))],
        }
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, hits: &mut HitMap) {
        let [body, hints] =
            Layout::vertical([Constraint::Min(5), Constraint::Length(1)]).areas(area);
        let title = if self.plan_name.is_empty() {
            "Set up run".to_string()
        } else {
            format!("Set up run: {}", self.plan_name)
        };
        let block = pane_block(&title, true);
        let inner = block.inner(body);
        frame.render_widget(block, body);
        hits.add(body, Hit::Pane(0));

        let visible = inner.height as usize;
        self.offset = clamp_scroll(self.cursor, self.rows.len(), visible, self.offset);
        let mut lines = Vec::new();
        for (i, row) in self.rows.iter().enumerate().skip(self.offset).take(visible) {
            let is_cursor = i == self.cursor;
            let line = match row {
                Row::Heading(text) => Line::from(Span::styled(text.clone(), theme::title())),
                Row::Start => Line::from(Span::styled(
                    "  [ Start run ]",
                    if is_cursor {
                        theme::selected().fg(theme::ACCENT)
                    } else {
                        theme::key()
                    },
                )),
                _ => {
                    let label = match row {
                        Row::Guard(GuardField::TempMax) => "Temperature cap (°C)".to_string(),
                        Row::Guard(GuardField::TimeMax) => "Time cap per test".to_string(),
                        Row::Guard(GuardField::Cooldown) => {
                            "Cooldown between tests (s)".to_string()
                        }
                        Row::Guard(GuardField::StopOnFail) => "Stop on first failure".to_string(),
                        Row::Profile { .. } => "Profile".to_string(),
                        Row::Param { spec, .. } => spec.label.to_string(),
                        Row::Repeat { .. } => "Repeat".to_string(),
                        _ => String::new(),
                    };
                    let help = match row {
                        Row::Param { spec, .. } => spec.help.to_string(),
                        Row::Repeat { .. } => "Run this test this many times in a row".to_string(),
                        Row::Profile { .. } => "Space cycles saved parameter sets".to_string(),
                        _ => String::new(),
                    };
                    let value = match (&self.editing, is_cursor) {
                        (Some(input), true) => input.line(true),
                        _ => {
                            let text = self.value_text(row);
                            let text = if text.is_empty() {
                                "(required)".to_string()
                            } else {
                                text
                            };
                            Line::from(Span::styled(text, Style::default().fg(theme::INFO)))
                        }
                    };
                    let mut spans = vec![Span::raw(format!("  {label:<28} "))];
                    spans.extend(value.spans);
                    let pad = 14usize.saturating_sub(value_width(&self.value_text(row)));
                    spans.push(Span::raw(" ".repeat(pad)));
                    match &self.errors[i] {
                        Some(err) => spans.push(Span::styled(
                            format!(" {err}"),
                            Style::default().fg(theme::BAD),
                        )),
                        None => spans.push(Span::styled(format!(" {help}"), theme::help())),
                    }
                    let mut line = Line::from(spans);
                    if is_cursor {
                        line = line.style(theme::selected());
                    }
                    line
                }
            };
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

fn value_width(text: &str) -> usize {
    text.chars().count()
}
