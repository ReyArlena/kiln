//! Live view of a run in progress.

use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Gauge, Paragraph, Wrap};

use crate::config::GraphsConfig;
use crate::engine::sampler::SensorInfo;
use crate::engine::test::{GraphSource, GraphSpec};
use crate::engine::util::format_duration_secs;
use crate::engine::{Event, Plan, Sample, Status};

use super::super::input::{Action, Hit, HitMap};
use super::super::theme;
use super::super::widgets::{Modal, draw_key_hints, pane_block};
use super::Effect;

const PANE_LOG: usize = 0;
const PANE_DMESG: usize = 1;
const MAX_LINES: usize = 3000;
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
/// Bottom of temperature graphs, in Celsius.
const TEMP_FLOOR_C: f64 = 30.0;
/// Most points kept per graph.
const MAX_POINTS: usize = 600;
/// Widths of the value and label columns right of each graph.
const VALUE_WIDTH: u16 = 7;
const LABEL_WIDTH: u16 = 7;

/// One live graph: its spec plus the points collected so far.
struct Series {
    spec: GraphSpec,
    points: Vec<Option<f64>>,
    latest: Option<f64>,
}

impl Series {
    fn new(spec: GraphSpec) -> Self {
        Self {
            spec,
            points: Vec::new(),
            latest: None,
        }
    }

    /// Read this series' value from a sample and the latest metrics.
    fn value_from(
        &self,
        sample: &Sample,
        metrics: &BTreeMap<String, f64>,
        mem_total: u64,
    ) -> Option<f64> {
        match &self.spec.source {
            GraphSource::Sensor(id) if id == "primary" => sample.primary_temp_c.map(|t| t as f64),
            GraphSource::Sensor(id) => sample
                .temps
                .iter()
                .find(|(s, _)| s == id)
                .map(|(_, t)| *t as f64),
            GraphSource::CpuFreq => sample.freq_mhz.map(|f| f as f64 / 1000.0),
            GraphSource::CpuPercent => sample.cpu_percent.map(|p| p as f64),
            GraphSource::MemoryPercent => {
                (mem_total > 0).then(|| sample.mem_used_bytes as f64 * 100.0 / mem_total as f64)
            }
            GraphSource::CpuPower => sample.cpu_power_w.map(|w| w as f64),
            GraphSource::GpuPower => sample.gpu_power_w.map(|w| w as f64),
            GraphSource::Metric(key) => metrics.get(key).copied(),
        }
    }

    /// The value range the graph is drawn over. Temperatures run from 30°C to
    /// the cap, percentages 0 to 100, everything else 0 to the highest seen.
    fn range(&self, temp_cap: Option<f32>) -> (f64, f64) {
        match self.spec.source {
            GraphSource::Sensor(_) => {
                let cap = if self.spec.label == "CPU" {
                    temp_cap.map(|c| c as f64)
                } else {
                    Some(85.0)
                };
                (TEMP_FLOOR_C, cap.unwrap_or(100.0).max(TEMP_FLOOR_C + 10.0))
            }
            GraphSource::CpuPercent | GraphSource::MemoryPercent => (0.0, 100.0),
            GraphSource::CpuFreq
            | GraphSource::CpuPower
            | GraphSource::GpuPower
            | GraphSource::Metric(_) => {
                let max = self
                    .points
                    .iter()
                    .flatten()
                    .copied()
                    .fold(0.0_f64, f64::max);
                (0.0, if max > 0.0 { max } else { 1.0 })
            }
        }
    }

    fn push(&mut self, value: Option<f64>) {
        if value.is_some() {
            self.latest = value;
        }
        self.points.push(value);
        if self.points.len() > MAX_POINTS {
            self.points.remove(0);
        }
    }

    fn label_value(&self) -> String {
        match (self.latest, &self.spec.source) {
            (None, _) => "-".into(),
            (Some(v), GraphSource::Sensor(_)) => format!("{v:.0}°C"),
            (Some(v), GraphSource::CpuFreq) => format!("{v:.1}"),
            (Some(v), GraphSource::CpuPower | GraphSource::GpuPower) => format!("{v:.0}W"),
            (Some(v), GraphSource::CpuPercent | GraphSource::MemoryPercent) => format!("{v:.0}%"),
            (Some(v), GraphSource::Metric(_)) => {
                if v.fract() == 0.0 {
                    format!("{}", v as i64)
                } else {
                    format!("{v:.1}")
                }
            }
        }
    }
}

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
    /// Set when the current test (or the run) ends so the clock stops.
    ended: Option<Instant>,
    expected_secs: Option<u64>,
    /// The always-on component graphs first, then the running test's metrics.
    graphs: Vec<Series>,
    /// How many of `graphs` are the always-on ones.
    base_graphs: usize,
    show_test_metrics: bool,
    mem_total: u64,
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

/// The always-on graphs, in display order, from settings and the sensors found.
fn component_graphs(
    config: &GraphsConfig,
    sensors: &[SensorInfo],
    has_cpu_power: bool,
    has_gpu_power: bool,
) -> Vec<GraphSpec> {
    let mut graphs = Vec::new();
    if config.cpu_temp {
        graphs.push(GraphSpec::sensor("CPU", "primary"));
    }
    if config.cpu_usage {
        graphs.push(GraphSpec::cpu_percent());
    }
    if config.cpu_freq {
        graphs.push(GraphSpec::cpu_freq());
    }
    if config.memory {
        graphs.push(GraphSpec::memory());
    }
    if config.cpu_power && has_cpu_power {
        graphs.push(GraphSpec::cpu_power());
    }
    if config.gpu_power && has_gpu_power {
        graphs.push(GraphSpec::gpu_power());
    }
    if config.nvme_temp {
        let mut nvmes: Vec<&SensorInfo> = sensors
            .iter()
            .filter(|s| s.chip.starts_with("nvme") && s.label == "Composite")
            .collect();
        nvmes.sort_by(|a, b| a.chip.cmp(&b.chip));
        for s in nvmes {
            graphs.push(GraphSpec::sensor(&s.chip.to_ascii_uppercase(), &s.id()));
        }
    }
    if config.gpu_temp
        && let Some(s) = sensors
            .iter()
            .find(|s| s.id() == "amdgpu/edge" || s.id() == "nouveau/temp1")
    {
        graphs.push(GraphSpec::sensor("GPU", &s.id()));
    }
    graphs
}

impl DashboardScreen {
    /// The temperature cap of the plan being run, for the header.
    pub fn temp_cap(&self) -> Option<f32> {
        self.temp_cap
    }

    pub fn empty() -> Self {
        Self::new(
            &Plan::new("", Default::default()),
            &GraphsConfig::default(),
            &[],
            false,
            false,
            0,
        )
    }

    pub fn new(
        plan: &Plan,
        config: &GraphsConfig,
        sensors: &[SensorInfo],
        has_cpu_power: bool,
        has_gpu_power: bool,
        mem_total: u64,
    ) -> Self {
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
        let graphs: Vec<Series> = component_graphs(config, sensors, has_cpu_power, has_gpu_power)
            .into_iter()
            .map(Series::new)
            .collect();
        Self {
            tests,
            current: None,
            current_name: String::new(),
            started: Instant::now(),
            ended: None,
            expected_secs: None,
            base_graphs: graphs.len(),
            graphs,
            show_test_metrics: config.test_metrics,
            mem_total,
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
                graphs,
                ..
            } => {
                self.current = Some(*index);
                self.current_name = name.clone();
                self.started = Instant::now();
                self.ended = None;
                self.expected_secs = *expected_secs;
                // Component graphs keep their history across tests; the test's own
                // metric series are replaced.
                self.graphs.truncate(self.base_graphs);
                if self.show_test_metrics {
                    self.graphs.extend(graphs.iter().cloned().map(Series::new));
                }
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
                self.ended = Some(Instant::now());
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
                for series in &mut self.graphs {
                    let value = series.value_from(s, &self.metrics, self.mem_total);
                    series.push(value);
                }
            }
            Event::RunFinished(_) => {
                self.finished = true;
                self.ended.get_or_insert_with(Instant::now);
            }
        }
    }

    /// Seconds since the current test started, frozen once it ends.
    fn elapsed_secs(&self) -> u64 {
        self.ended
            .unwrap_or_else(Instant::now)
            .duration_since(self.started)
            .as_secs()
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
        let elapsed = format_duration_secs(self.elapsed_secs());
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
        // Gauge + one row per graph + metrics line + borders.
        let top_height = (self.graphs.len() as u16 + 4).max(8);
        let [top, bottom] =
            Layout::vertical([Constraint::Length(top_height), Constraint::Min(5)]).areas(body);
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
        let block = pane_block(
            &if self.finished {
                "Finished".to_string()
            } else {
                format!("Now: {}", self.current_name)
            },
            false,
        );
        let inner = block.inner(progress_area);
        frame.render_widget(block, progress_area);
        let [gauge_area, graphs_area, metrics_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(self.graphs.len() as u16),
            Constraint::Min(1),
        ])
        .areas(inner);

        let elapsed = self.elapsed_secs();
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

        let rows =
            Layout::vertical(vec![Constraint::Length(1); self.graphs.len()]).split(graphs_area);
        let cap = self.temp_cap;
        for (series, row) in self.graphs.iter().zip(rows.iter()) {
            draw_series(frame, *row, series, cap);
        }

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

/// One graph row: label on the left, braille graph in the middle, latest
/// value on the right. Colour runs green to yellow to red as the value
/// climbs through the series' range, like btop.
fn draw_series(frame: &mut Frame, row: Rect, series: &Series, temp_cap: Option<f32>) {
    // Graph on the left, then the value, then its label: `⣿⣿⣿  71°C CPU`.
    let [graph, value_area, label_area] = Layout::horizontal([
        Constraint::Min(1),
        Constraint::Length(VALUE_WIDTH),
        Constraint::Length(LABEL_WIDTH),
    ])
    .areas(row);
    let (floor, ceiling) = series.range(temp_cap);
    let normalise = |v: f64| ((v - floor) / (ceiling - floor)).clamp(0.0, 1.0);

    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" {}", series.spec.label),
            theme::help(),
        )),
        label_area,
    );

    // Two points per cell: the braille left and right dot columns.
    let cells = graph.width as usize;
    let needed = cells * 2;
    let recent: Vec<Option<f64>> = series
        .points
        .iter()
        .rev()
        .take(needed)
        .rev()
        .copied()
        .collect();
    let mut points: Vec<Option<f64>> = vec![None; needed.saturating_sub(recent.len())];
    points.extend(recent.into_iter().map(|p| p.map(normalise)));
    let spans: Vec<Span> = points
        .chunks(2)
        .map(|pair| {
            let left = pair.first().copied().flatten();
            let right = pair.get(1).copied().flatten();
            let symbol = braille_cell(left, right);
            let level = left
                .into_iter()
                .chain(right)
                .fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))));
            Span::styled(
                symbol.to_string(),
                Style::default().fg(level.map(gradient).unwrap_or(theme::DIM)),
            )
        })
        .collect();
    frame.render_widget(Paragraph::new(Line::from(spans)), graph);

    let value_style = Style::default().fg(series
        .latest
        .map(|v| gradient(normalise(v)))
        .unwrap_or(theme::DIM));
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(
                "{:>width$}",
                series.label_value(),
                width = VALUE_WIDTH as usize
            ),
            value_style,
        )),
        value_area,
    );
}

/// Braille dot bits for the left and right columns, bottom row first.
const LEFT_DOTS: [u32; 4] = [0x40, 0x04, 0x02, 0x01];
const RIGHT_DOTS: [u32; 4] = [0x80, 0x20, 0x10, 0x08];

/// A braille character whose two columns are filled from the bottom to the
/// given levels (0.0 to 1.0). `None` leaves a column empty.
fn braille_cell(left: Option<f64>, right: Option<f64>) -> char {
    let mut bits = 0x2800u32;
    for (value, dots) in [(left, LEFT_DOTS), (right, RIGHT_DOTS)] {
        let Some(v) = value else { continue };
        // At least one dot once a value exists, so a flat zero line still shows.
        let filled = ((v * 4.0).round() as usize).clamp(1, 4);
        for dot in dots.iter().take(filled) {
            bits |= dot;
        }
    }
    char::from_u32(bits).unwrap_or(' ')
}

/// Green at 0, yellow at 0.5, red at 1.
fn gradient(t: f64) -> Color {
    let t = t.clamp(0.0, 1.0);
    let (r, g) = if t < 0.5 {
        let k = t / 0.5;
        ((60.0 + (235.0 - 60.0) * k) as u8, 200u8)
    } else {
        let k = (t - 0.5) / 0.5;
        (235u8, (200.0 - (200.0 - 60.0) * k) as u8)
    };
    Color::Rgb(r, g, 60)
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
    let style = if crate::engine::util::looks_like_problem(text) {
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
    fn braille_cells_fill_from_the_bottom() {
        assert_eq!(braille_cell(None, None), '\u{2800}');
        assert_eq!(braille_cell(Some(1.0), Some(1.0)), '\u{28ff}');
        // Bottom dot only on the left column.
        assert_eq!(braille_cell(Some(0.1), None), '\u{2840}');
        // Two dots left, all four right.
        assert_eq!(
            braille_cell(Some(0.5), Some(1.0)),
            char::from_u32(0x2800 | 0x40 | 0x04 | 0xF8).unwrap()
        );
    }

    #[test]
    fn long_lines_wrap_and_empty_lines_survive() {
        assert_eq!(wrap_line("abcdefgh", 3), vec!["abc", "def", "gh"]);
        assert_eq!(wrap_line("", 3), vec![""]);
        assert_eq!(wrap_line("ab", 10), vec!["ab"]);
    }
}
