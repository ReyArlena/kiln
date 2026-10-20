//! Live view of a run in progress.

use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Gauge, Paragraph, Sparkline, Wrap};

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
/// Width of the label column left of each graph.
const GUTTER: u16 = 14;

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

    fn is_temperature(&self) -> bool {
        matches!(self.spec.source, GraphSource::Sensor(_))
    }

    /// Read this series' value from a sample and the latest metrics.
    fn value_from(&self, sample: &Sample, metrics: &BTreeMap<String, f64>) -> Option<f64> {
        match &self.spec.source {
            GraphSource::Sensor(id) => sample
                .temps
                .iter()
                .find(|(s, _)| s == id)
                .map(|(_, t)| *t as f64),
            GraphSource::CpuFreq => sample.freq_mhz.map(|f| f as f64 / 1000.0),
            GraphSource::CpuPercent => sample.cpu_percent.map(|p| p as f64),
            GraphSource::Metric(key) => metrics.get(key).copied(),
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
            (Some(v), GraphSource::CpuPercent) => format!("{v:.0}%"),
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
    /// CPU temperature first, then whatever the running test asked for.
    graphs: Vec<Series>,
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

/// The CPU package temperature, graphed for every test.
fn cpu_temp_graph() -> GraphSpec {
    GraphSpec::sensor("CPU", "primary")
}

impl DashboardScreen {
    /// The temperature cap of the plan being run, for the header.
    pub fn temp_cap(&self) -> Option<f32> {
        self.temp_cap
    }

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
            ended: None,
            expected_secs: None,
            graphs: vec![Series::new(cpu_temp_graph())],
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
                // Keep the CPU history across tests; swap the rest for this test's picks.
                self.graphs.truncate(1);
                self.graphs.extend(graphs.iter().cloned().map(Series::new));
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
                for (i, series) in self.graphs.iter_mut().enumerate() {
                    let value = if i == 0 {
                        s.primary_temp_c.map(|t| t as f64)
                    } else {
                        series.value_from(s, &self.metrics)
                    };
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

/// One graph row: label and latest value in the gutter, sparkline filling the rest,
/// newest point at the right edge.
fn draw_series(frame: &mut Frame, row: Rect, series: &Series, temp_cap: Option<f32>) {
    let [gutter, graph] =
        Layout::horizontal([Constraint::Length(GUTTER), Constraint::Min(1)]).areas(row);
    let value_style = match (series.is_temperature(), series.latest) {
        (true, Some(v)) => theme::temp(
            v as f32,
            if series.spec.label == "CPU" {
                temp_cap
            } else {
                Some(80.0)
            },
        ),
        _ => theme::number(),
    };
    let label = Line::from(vec![
        Span::styled(format!("{:<5}", series.spec.label), theme::dim()),
        Span::styled(format!("{:>7} ", series.label_value()), value_style),
    ]);
    frame.render_widget(Paragraph::new(label), gutter);

    // Scale every series to 0..1000 so one sparkline call fits all of them.
    let (floor, ceiling) = match series.spec.source {
        GraphSource::Sensor(_) => {
            let cap = if series.spec.label == "CPU" {
                temp_cap.map(|c| c as f64)
            } else {
                Some(85.0)
            };
            (TEMP_FLOOR_C, cap.unwrap_or(100.0).max(TEMP_FLOOR_C + 10.0))
        }
        GraphSource::CpuPercent => (0.0, 100.0),
        GraphSource::CpuFreq | GraphSource::Metric(_) => {
            let max = series
                .points
                .iter()
                .flatten()
                .copied()
                .fold(0.0_f64, f64::max);
            (0.0, if max > 0.0 { max } else { 1.0 })
        }
    };
    let width = graph.width as usize;
    let recent = series.points.iter().rev().take(width).rev();
    let mut data: Vec<Option<u64>> =
        vec![None; width.saturating_sub(series.points.len().min(width))];
    data.extend(
        recent.map(|p| {
            p.map(|v| (((v - floor) / (ceiling - floor)).clamp(0.0, 1.0) * 1000.0) as u64)
        }),
    );
    frame.render_widget(
        Sparkline::default()
            .data(data)
            .max(1000)
            .style(Style::default().fg(theme::ACCENT)),
        graph,
    );
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
