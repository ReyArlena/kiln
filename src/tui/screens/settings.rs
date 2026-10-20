//! App settings plus developer tools. Every change is saved immediately.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::AppState;
use crate::config::SensorChoice;
use crate::engine::sampler::discover_sensors;
use crate::engine::util::{format_duration_secs, parse_duration_secs};

use super::super::input::{Action, Hit, HitMap};
use super::super::theme;
use super::super::widgets::{Modal, TextInput, clamp_scroll, draw_key_hints, pane_block};
use super::{Effect, Screen};

#[derive(Clone, Copy, PartialEq)]
enum Item {
    Heading(&'static str),
    TempMax,
    TimeMax,
    Cooldown,
    StopOnFail,
    Sensor,
    SampleInterval,
    Mouse,
    ConfirmDestructive,
    RunsToKeep,
    BarEnabled,
    BarCpuTemp,
    BarCpuUsage,
    BarCpuFreq,
    BarMemory,
    BarNvme,
    BarGpu,
    BarCpuPower,
    BarGpuPower,
    GraphCpuTemp,
    GraphCpuUsage,
    GraphCpuFreq,
    GraphMemory,
    GraphCpuPower,
    GraphGpuPower,
    GraphNvme,
    GraphGpu,
    GraphTestMetrics,
    DevReset,
    DevDeleteRuns,
    DevRestart,
    DevExit,
    Paths,
}

const ITEMS: &[Item] = &[
    Item::Heading("Default guards for new runs"),
    Item::TempMax,
    Item::TimeMax,
    Item::Cooldown,
    Item::StopOnFail,
    Item::Heading("Sensors"),
    Item::Sensor,
    Item::SampleInterval,
    Item::Heading("Interface"),
    Item::Mouse,
    Item::ConfirmDestructive,
    Item::RunsToKeep,
    Item::Heading("Status bar"),
    Item::BarEnabled,
    Item::BarCpuTemp,
    Item::BarCpuUsage,
    Item::BarCpuFreq,
    Item::BarMemory,
    Item::BarNvme,
    Item::BarGpu,
    Item::BarCpuPower,
    Item::BarGpuPower,
    Item::Heading("Dashboard graphs"),
    Item::GraphCpuTemp,
    Item::GraphCpuUsage,
    Item::GraphCpuFreq,
    Item::GraphMemory,
    Item::GraphCpuPower,
    Item::GraphGpuPower,
    Item::GraphNvme,
    Item::GraphGpu,
    Item::GraphTestMetrics,
    Item::Heading("Developer tools"),
    Item::DevReset,
    Item::DevDeleteRuns,
    Item::DevRestart,
    Item::DevExit,
    Item::Heading("Files"),
    Item::Paths,
];

pub struct SettingsScreen {
    cursor: usize,
    offset: usize,
    editing: Option<TextInput>,
    error: Option<String>,
    sensors: Vec<String>,
}

impl SettingsScreen {
    pub fn new() -> Self {
        Self {
            cursor: 1,
            offset: 0,
            editing: None,
            error: None,
            sensors: discover_sensors().iter().map(|s| s.id()).collect(),
        }
    }

    pub fn is_editing(&self) -> bool {
        self.editing.is_some()
    }

    pub fn key_hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.editing.is_some() {
            vec![("Enter", "apply"), ("Esc", "cancel edit")]
        } else {
            vec![
                ("Enter", "edit / activate"),
                ("Space", "toggle / cycle"),
                ("Esc", "back"),
                ("?", "help"),
            ]
        }
    }

    fn label(item: Item) -> &'static str {
        match item {
            Item::Heading(h) => h,
            Item::TempMax => "Temperature cap (°C)",
            Item::TimeMax => "Time cap per test",
            Item::Cooldown => "Cooldown between tests (s)",
            Item::StopOnFail => "Stop on first failure",
            Item::Sensor => "Guard sensor",
            Item::SampleInterval => "Sample interval (ms)",
            Item::Mouse => "Mouse support (restart to apply)",
            Item::ConfirmDestructive => "Typed confirm before wiping",
            Item::RunsToKeep => "Runs to keep (0 = all)",
            Item::BarEnabled => "Show status bar",
            Item::BarCpuTemp => "  CPU temperature",
            Item::BarCpuUsage => "  CPU usage",
            Item::BarCpuFreq => "  CPU frequency",
            Item::BarMemory => "  Memory",
            Item::BarNvme => "  NVMe temperatures",
            Item::BarGpu => "  GPU temperature",
            Item::BarCpuPower => "  CPU power (needs root)",
            Item::BarGpuPower => "  GPU power",
            Item::GraphCpuTemp => "  CPU temperature",
            Item::GraphCpuUsage => "  CPU usage",
            Item::GraphCpuFreq => "  CPU frequency",
            Item::GraphMemory => "  Memory",
            Item::GraphCpuPower => "  CPU power (needs root)",
            Item::GraphGpuPower => "  GPU power",
            Item::GraphNvme => "  NVMe temperatures",
            Item::GraphGpu => "  GPU temperature",
            Item::GraphTestMetrics => "  Test metrics (e.g. MB/s)",
            Item::DevReset => "Clear all settings (back to defaults)",
            Item::DevDeleteRuns => "Delete all run history",
            Item::DevRestart => "Restart app",
            Item::DevExit => "Exit app",
            Item::Paths => "",
        }
    }

    fn value(item: Item, state: &AppState) -> String {
        let c = &state.config;
        let yes_no = |b: bool| if b { "yes" } else { "no" }.to_string();
        match item {
            Item::TempMax => c
                .guards
                .temp_max_c
                .map(|t| format!("{t:.0}"))
                .unwrap_or_else(|| "off".into()),
            Item::TimeMax => c
                .guards
                .time_max_secs
                .map(format_duration_secs)
                .unwrap_or_else(|| "off".into()),
            Item::Cooldown => c.guards.cooldown_secs.to_string(),
            Item::StopOnFail => yes_no(c.guards.stop_on_fail),
            Item::Sensor => match &c.sensor {
                SensorChoice::Auto => format!(
                    "auto ({})",
                    state
                        .sampler()
                        .primary
                        .map(|s| s.id())
                        .unwrap_or_else(|| "none".into())
                ),
                SensorChoice::Named(id) => id.clone(),
            },
            Item::SampleInterval => c.sample_interval_ms.to_string(),
            Item::Mouse => yes_no(c.mouse),
            Item::ConfirmDestructive => yes_no(c.confirm_destructive),
            Item::RunsToKeep => c.runs_to_keep.to_string(),
            Item::BarEnabled => yes_no(c.status_bar.enabled),
            Item::BarCpuTemp => yes_no(c.status_bar.cpu_temp),
            Item::BarCpuUsage => yes_no(c.status_bar.cpu_usage),
            Item::BarCpuFreq => yes_no(c.status_bar.cpu_freq),
            Item::BarMemory => yes_no(c.status_bar.memory),
            Item::BarNvme => yes_no(c.status_bar.nvme_temp),
            Item::BarGpu => yes_no(c.status_bar.gpu_temp),
            Item::BarCpuPower => yes_no(c.status_bar.cpu_power),
            Item::BarGpuPower => yes_no(c.status_bar.gpu_power),
            Item::GraphCpuTemp => yes_no(c.graphs.cpu_temp),
            Item::GraphCpuUsage => yes_no(c.graphs.cpu_usage),
            Item::GraphCpuFreq => yes_no(c.graphs.cpu_freq),
            Item::GraphMemory => yes_no(c.graphs.memory),
            Item::GraphCpuPower => yes_no(c.graphs.cpu_power),
            Item::GraphGpuPower => yes_no(c.graphs.gpu_power),
            Item::GraphNvme => yes_no(c.graphs.nvme_temp),
            Item::GraphGpu => yes_no(c.graphs.gpu_temp),
            Item::GraphTestMetrics => yes_no(c.graphs.test_metrics),
            _ => String::new(),
        }
    }

    fn is_text(item: Item) -> bool {
        matches!(
            item,
            Item::TempMax
                | Item::TimeMax
                | Item::Cooldown
                | Item::SampleInterval
                | Item::RunsToKeep
        )
    }

    fn is_toggle(item: Item) -> bool {
        matches!(
            item,
            Item::StopOnFail
                | Item::Sensor
                | Item::Mouse
                | Item::ConfirmDestructive
                | Item::BarEnabled
                | Item::BarCpuTemp
                | Item::BarCpuUsage
                | Item::BarCpuFreq
                | Item::BarMemory
                | Item::BarNvme
                | Item::BarGpu
                | Item::BarCpuPower
                | Item::BarGpuPower
                | Item::GraphCpuTemp
                | Item::GraphCpuUsage
                | Item::GraphCpuFreq
                | Item::GraphMemory
                | Item::GraphCpuPower
                | Item::GraphGpuPower
                | Item::GraphNvme
                | Item::GraphGpu
                | Item::GraphTestMetrics
        )
    }

    fn selectable(idx: usize) -> bool {
        !matches!(
            ITEMS.get(idx),
            Some(Item::Heading(_)) | Some(Item::Paths) | None
        )
    }

    fn move_cursor(&mut self, delta: i32) {
        let mut idx = self.cursor as i32;
        loop {
            idx += delta;
            if idx < 0 || idx >= ITEMS.len() as i32 {
                return;
            }
            if Self::selectable(idx as usize) {
                self.cursor = idx as usize;
                return;
            }
        }
    }

    pub fn handle(&mut self, action: Action, state: &mut AppState, hits: &HitMap) -> Vec<Effect> {
        if let Some(input) = &mut self.editing {
            if let Action::Key(key) = action {
                input.handle_key(key);
            }
            return Vec::new();
        }
        match action {
            Action::Up => self.move_cursor(-1),
            Action::Down => self.move_cursor(1),
            Action::PageUp => (0..6).for_each(|_| self.move_cursor(-1)),
            Action::PageDown => (0..6).for_each(|_| self.move_cursor(1)),
            Action::Home => {
                self.cursor = 0;
                self.move_cursor(1);
            }
            Action::End => {
                self.cursor = ITEMS.len() - 1;
                self.move_cursor(-1);
            }
            Action::Toggle => return self.activate(state, true),
            Action::Confirm => return self.activate(state, false),
            Action::Back => return vec![Effect::Goto(Screen::Picker)],
            Action::Click { x, y } => {
                if let Some(Hit::Row { index, .. }) = hits.find(x, y)
                    && Self::selectable(*index)
                {
                    if self.cursor == *index {
                        return self.activate(state, true);
                    }
                    self.cursor = *index;
                }
            }
            Action::Scroll { up, .. } => self.move_cursor(if up { -1 } else { 1 }),
            _ => {}
        }
        Vec::new()
    }

    /// Space or Enter on the current item.
    fn activate(&mut self, state: &mut AppState, is_space: bool) -> Vec<Effect> {
        let item = ITEMS[self.cursor];
        let c = &mut state.config;
        if Self::is_toggle(item) {
            match item {
                Item::StopOnFail => c.guards.stop_on_fail = !c.guards.stop_on_fail,
                Item::Mouse => c.mouse = !c.mouse,
                Item::ConfirmDestructive => c.confirm_destructive = !c.confirm_destructive,
                Item::BarEnabled => c.status_bar.enabled = !c.status_bar.enabled,
                Item::BarCpuTemp => c.status_bar.cpu_temp = !c.status_bar.cpu_temp,
                Item::BarCpuUsage => c.status_bar.cpu_usage = !c.status_bar.cpu_usage,
                Item::BarCpuFreq => c.status_bar.cpu_freq = !c.status_bar.cpu_freq,
                Item::BarMemory => c.status_bar.memory = !c.status_bar.memory,
                Item::BarNvme => c.status_bar.nvme_temp = !c.status_bar.nvme_temp,
                Item::BarGpu => c.status_bar.gpu_temp = !c.status_bar.gpu_temp,
                Item::BarCpuPower => c.status_bar.cpu_power = !c.status_bar.cpu_power,
                Item::BarGpuPower => c.status_bar.gpu_power = !c.status_bar.gpu_power,
                Item::GraphCpuTemp => c.graphs.cpu_temp = !c.graphs.cpu_temp,
                Item::GraphCpuUsage => c.graphs.cpu_usage = !c.graphs.cpu_usage,
                Item::GraphCpuFreq => c.graphs.cpu_freq = !c.graphs.cpu_freq,
                Item::GraphMemory => c.graphs.memory = !c.graphs.memory,
                Item::GraphCpuPower => c.graphs.cpu_power = !c.graphs.cpu_power,
                Item::GraphGpuPower => c.graphs.gpu_power = !c.graphs.gpu_power,
                Item::GraphNvme => c.graphs.nvme_temp = !c.graphs.nvme_temp,
                Item::GraphGpu => c.graphs.gpu_temp = !c.graphs.gpu_temp,
                Item::GraphTestMetrics => c.graphs.test_metrics = !c.graphs.test_metrics,
                Item::Sensor => {
                    // Cycle: auto, then each discovered sensor.
                    let next = match &c.sensor {
                        SensorChoice::Auto => self
                            .sensors
                            .first()
                            .cloned()
                            .map(SensorChoice::Named)
                            .unwrap_or(SensorChoice::Auto),
                        SensorChoice::Named(id) => {
                            match self.sensors.iter().position(|s| s == id) {
                                Some(i) if i + 1 < self.sensors.len() => {
                                    SensorChoice::Named(self.sensors[i + 1].clone())
                                }
                                _ => SensorChoice::Auto,
                            }
                        }
                    };
                    c.sensor = next;
                }
                _ => {}
            }
            return vec![Effect::SaveConfig];
        }
        if Self::is_text(item) {
            let current = Self::value(item, state);
            self.editing = Some(TextInput::with_value(&current));
            self.error = None;
            return Vec::new();
        }
        if is_space {
            return Vec::new();
        }
        match item {
            Item::DevReset => vec![Effect::Modal(Modal::confirm(
                "Clear settings",
                "Delete the settings file and return to defaults?",
                "reset-config",
            ))],
            Item::DevDeleteRuns => vec![Effect::Modal(Modal::confirm(
                "Delete history",
                "Delete every saved run and log?",
                "delete-runs",
            ))],
            Item::DevRestart => vec![Effect::Restart],
            Item::DevExit => vec![Effect::Quit],
            _ => Vec::new(),
        }
    }

    pub fn commit_edit(&mut self, state: &mut AppState) -> Vec<Effect> {
        let Some(input) = self.editing.take() else {
            return Vec::new();
        };
        let text = input.value.trim().to_string();
        let c = &mut state.config;
        let result: Result<(), String> = match ITEMS[self.cursor] {
            Item::TempMax => {
                if text.eq_ignore_ascii_case("off") || text.is_empty() {
                    c.guards.temp_max_c = None;
                    Ok(())
                } else {
                    text.parse::<f32>()
                        .map(|v| c.guards.temp_max_c = Some(v))
                        .map_err(|_| "number or `off`".to_string())
                }
            }
            Item::TimeMax => {
                if text.eq_ignore_ascii_case("off") || text.is_empty() {
                    c.guards.time_max_secs = None;
                    Ok(())
                } else {
                    parse_duration_secs(&text)
                        .map(|v| c.guards.time_max_secs = Some(v))
                        .ok_or_else(|| "duration like 30m, or `off`".to_string())
                }
            }
            Item::Cooldown => text
                .parse::<u64>()
                .map(|v| c.guards.cooldown_secs = v)
                .map_err(|_| "whole number".to_string()),
            Item::SampleInterval => text
                .parse::<u64>()
                .ok()
                .filter(|v| *v >= 200)
                .map(|v| c.sample_interval_ms = v)
                .ok_or_else(|| "at least 200".to_string()),
            Item::RunsToKeep => text
                .parse::<usize>()
                .map(|v| c.runs_to_keep = v)
                .map_err(|_| "whole number".to_string()),
            _ => Ok(()),
        };
        match result {
            Ok(()) => vec![Effect::SaveConfig],
            Err(e) => {
                self.error = Some(e);
                Vec::new()
            }
        }
    }

    pub fn cancel_edit(&mut self) {
        self.editing = None;
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, state: &AppState, hits: &mut HitMap) {
        let [body, hints] =
            Layout::vertical([Constraint::Min(5), Constraint::Length(1)]).areas(area);
        let block = pane_block("Settings", true);
        let inner = block.inner(body);
        frame.render_widget(block, body);
        hits.add(body, Hit::Pane(0));

        let visible = inner.height as usize;
        self.offset = clamp_scroll(self.cursor, ITEMS.len(), visible, self.offset);
        let mut lines = Vec::new();
        for (i, item) in ITEMS.iter().enumerate().skip(self.offset).take(visible) {
            let is_cursor = i == self.cursor;
            let line = match item {
                Item::Heading(h) => Line::from(Span::styled(*h, theme::title())),
                Item::Paths => Line::from(Span::styled(
                    format!(
                        "  config {}   runs {}",
                        state.paths.config_file.display(),
                        state.paths.runs_dir.display()
                    ),
                    theme::dim(),
                )),
                Item::DevReset | Item::DevDeleteRuns | Item::DevRestart | Item::DevExit => {
                    let mut line = Line::from(vec![
                        Span::raw("  "),
                        Span::styled(
                            format!("[ {} ]", Self::label(*item)),
                            Style::default().fg(
                                if matches!(item, Item::DevReset | Item::DevDeleteRuns) {
                                    theme::BAD
                                } else {
                                    theme::ACCENT
                                },
                            ),
                        ),
                    ]);
                    if is_cursor {
                        line = line.style(theme::selected());
                    }
                    line
                }
                _ => {
                    let mut spans = vec![Span::raw(format!("  {:<36} ", Self::label(*item)))];
                    match (&self.editing, is_cursor) {
                        (Some(input), true) => spans.extend(input.line(true).spans),
                        _ => spans.push(Span::styled(
                            Self::value(*item, state),
                            Style::default().fg(theme::INFO),
                        )),
                    }
                    if is_cursor && let Some(err) = &self.error {
                        spans.push(Span::styled(
                            format!("   {err}"),
                            Style::default().fg(theme::BAD),
                        ));
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
